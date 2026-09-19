//! envcall 权柄轴（class 7 `PieCall`）—— 许可的生死与流动，十二个操作。
//!
//! 与数据轴（`envcall/mail.rs`）的分界：本模块**不搬运载荷**——传的是许可，
//! 内容走 class 5。三条轴（资源 / 持有 / 转授）见 `crates/env/src/fid.rs`。
//!
//! # 按 token 取用：判据在核心，只有一处
//!
//! 两条路住在 `work::unit::gate::pie`，本轴与数据轴**共用**：
//!
//! - [`gate::locate`]：**只定位**（表里没有 → `Denied`），死活与权限都不看。给"要那一枚
//!   **本身**"的动词：`Release`（封印后仍须能收尾）、`Reserve`（`owner` 随资源不变）、
//!   `Accord` 的源枚（它自己的四道闸在 `gate::accord`）、`Shut`（撤的是自己那张 PTE，
//!   封印也得撤得掉）、`Seal` 与 `Narrow` 的头一步。
//! - [`gate::accede`]：**定位 + 死活 + 权限**（已封印 → `Dead`；权不够 → `Denied`）。
//!   给真要用它的动词：`Open` 与数据轴的 `Push`/`Pull`/`Wait`/`Hush`/`Ring`。
//!
//! **顺序只有 `accede` 那一份**——同一个已封印的 token 不因动词换一个答案。这条曾是每个
//! 动词各自的纪律，于是漏成了两处：`Narrow` 把"覆盖子集"排在死活之前，数据轴四个动词把
//! "权限"排在死活之前（同一个已封印的 token，在两个轴、五个动词上答 `Denied`，别处答
//! `Dead`）。判据下沉之后，**"按动词分叉"在结构上写不出来了**——要分叉就得再手写一遍查找。
//!
//! [`usable`]（"被关住"那一维）仍住本层：它要摸**别人**的表，必须在放开本任务 `pies`
//! 之后判。故调用点的形状统一是"先 `accede`/`locate` 拿抄件 → 放锁 → 问 `usable`"。
//!
//! `ident` 的所有权移交与门面一致；本轴的操作都不换帧（无挂起）。

use alloc::sync::Arc;

use env::{Name, PieCall};

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::{self, AnyPie, GateError, Need, Permission, Pie, clear_heir};
use crate::work::unit::task::TaskIdent;

use super::subset_to_pte;

/// 记号（`UnsealHole` 收的那一格名字）一次能收多少字节——与 `env::wire::NAME_LEN` 同值：
/// 内核在入口把它拷进**栈上的定长缓冲**（解封路径不分配），上限因此必须是常量。
const NAME_LEN: usize = env::wire::NAME_LEN;

/// 一次权柄 envcall 的落点：本轴从不换帧，故只有一个变体。
///
/// 与数据轴的 `Outcome` 同名同形——门面按 `Park` 的有无便可判定「本操作是否可能
/// 换帧」。本轴**没有** `Park` 分支，这不是遗漏：权柄操作不挂起。
pub(crate) enum Outcome {
    /// 续跑当前任务，返回值已写入 a0/a1。
    Resume,
}

/// 权柄轴的十二个操作。返回 `None` = 本次调用不属于本轴（交还门面继续匹配）。
pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: PieCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    let _ = &ident;
    Some(match call {
        PieCall::UnsealHole { mark, len } => unseal_hole(frame, &ident, mark.get(), len),
        PieCall::UnsealPole { size } => unseal_pole(frame, size),
        PieCall::UnsealNole => unseal_nole(frame, ident),
        PieCall::Open { token } => open(frame, ident, token.get()),
        PieCall::Shut { token } => shut(frame, ident, token.get()),
        PieCall::Seal { token } => seal(frame, token.get()),
        PieCall::Accord { src, dst, subset } => accord(frame, src.get(), dst.get(), subset),
        PieCall::Narrow { token, subset } => narrow(frame, token.get(), subset),
        PieCall::Revoke { dst, token } => revoke(frame, dst.get(), token.get()),
        PieCall::Collect { index } => collect(frame, index),
        PieCall::Reserve { token, mark, cap } => {
            reserve(frame, &ident, token.get(), mark.get(), cap)
        }
        PieCall::Release { token } => release(frame, token.get()),
    })
}

/// 写回 a0（权柄操作的成功值：0 = 成功，负 = `GateError` 码）。
fn answer(frame: &mut TrapContext, r: Result<usize, GateError>) {
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(v) => v,
            Err(e) => e.code() as usize,
        },
    );
}

/// 写回 a0 + a1（两件返回；契约见 `crates/env` 的 `FromPair`）。错误路径只写 a0。
fn answer_pair(frame: &mut TrapContext, r: Result<(usize, usize), GateError>) {
    match r {
        Ok((v0, v1)) => {
            frame.gpr.set_x(Gprs::A0, v0);
            frame.gpr.set_x(Gprs::A1, v1);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
}

// 按 token 取用的两条路已下沉到核心：`gate::locate`（只定位）与 `gate::accede`
// （定位 + 死活 + 权限）。**判据顺序只有那一处**，本层只把**当前任务**递进去。
//
// 「被关住」闸（[`usable`]）仍住本层：它要摸**别人**的表，必须在放开本任务 `pies` 之后
// 判。故调用点的形状统一是"先 `accede`/`locate` 拿抄件 → 放锁 → 问 `usable`"。

/// 「被关住」闸——位与存活之外的**第三个判据维度**。
///
/// 读本地锚：锚空 ⇒ 放行；锚指向的那一枚**还在** ⇒ `Caged`（我把使用权交出去了，
/// 而接收方还活着）；已不在 ⇒ **清锚** ⇒ 放行。清锚是唯一的"解关"动作（没有独立
/// 动词）——交回、撤销、接收方死亡都只是让那一枚消失，而"消失"由这里读出来，
/// 故它必须回到**本任务表**里写（判据拿到的是抄件）。
///
/// 锚只有**独占资源**（源枚带 `ONLY`）的授出才写（见 `gate::accord`），故这道闸
/// 只在独占资源上生效——共享资源没有锚，永远放行。
///
/// 挂点（十个动词、九个调用点）：数据轴五个（`Push`/`Pull`/`Wait`/`Hush`/`Ring`——后两个
/// 共用 `with_bell` 那一处，故四格）、Pole 的两个（`open` 借映 / `shut` 撤映）、
/// `Accord` 的**源枚**、`Tole` 的两个（`Hang` 的成员枚 / `Await` 接组时的等待权）。
/// **不挂**查询与收场（`Reserve`/`Collect`/`Release`/`Revoke`/`Narrow`/`Seal`）；三个
/// `Unseal*` 也没有源枚可查——它们造的是新的一枚。
///
/// 锁序：核对要摸**别人**的表（L3），故必须**在放开本任务 `pies` 之后**调用；本函数
/// 自己逐任务取放，绝不嵌套。
pub(super) fn usable(pie: &AnyPie) -> Result<(), GateError> {
    let Some(h) = pie.heir().copied() else {
        return Ok(());
    };
    let held = muster(h.task)
        .and_then(|t| t.upgrade())
        .is_some_and(|t| t.pies.lock().iter().any(|p| p.token() == h.token));
    if held {
        return Err(GateError::Caged);
    }
    if let Some(task) = current().running_task() {
        clear_heir(&task, pie.token());
    }
    Ok(())
}

/// 解封 Hole：**先收记号**（有界拷入 + 按 `Name` 的解码面校验）→ 建资源实体 → 建门闩
/// （原始自持：无 sire）→ 落表。
///
/// 门闩持资源实体的强引用——**寿命即能力寿命**：最后一份消失时资源随之回收。
///
/// 原始自持枚带 `FETCH | STORE | VEST`：读写两支（能收能发）＋ 目标位（能再授出）。
/// **不带 `ONLY`**——用户态铸的资源都是共享的；"只允许一个使用者"是内核决定的事实
/// （设备 `reg` 段、组），只有那些创建点才给这一位。
///
/// 记号 = **这条路的名字**（`UnsealHole { mark, len }`）：`len == 0` / `len > NAME_LEN` /
/// 区间未映射 / 非法名（空、含 NUL、非 UTF-8）一律 `Denied`——**不截断、不 panic**。
/// 越界那一格先判、再拷：域那一段多长由 `len` 说了算，内核只在自己那 [`NAME_LEN`]
/// 字节的栈缓冲里收。拷贝走 `mail::copy_in`（整段验完才拷），且**在持 `pies` 锁之前**
/// ——它要过 `space.segments`（L2），持 L3 锁做反向嵌套会当场 panic。
fn unseal_hole(frame: &mut TrapContext, ident: &TaskIdent, buf: usize, len: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        if len == 0 || len > NAME_LEN {
            return Err(GateError::Denied);
        }
        // 记号先落栈缓冲、再校验：非法名到不了 `hole::meta`（那里只收已定型的 `Name`）。
        let mut raw = [0u8; NAME_LEN];
        if !mail::copy_in(&ident.team.space, &mut raw[..len], buf) {
            return Err(GateError::Denied);
        }
        let mark = Name::from_slice(&raw[..len]).map_err(|_| GateError::Denied)?;
        let task = current().running_task().ok_or(GateError::Denied)?;
        let meta = mail::hole::meta(task.ident.id, mark);
        let pie: Pie<mail::hole::HoleMeta> = gate::new_pie(
            meta,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        // **紧贴 push**：`pie` 是最后一步造的，drop 它即回收资源实体 ⇒ 失败就地退回。
        let mut pies = task.pies.lock();
        pies.try_reserve(1).map_err(|_| GateError::OoM)?;
        pies.push(AnyPie::Hole(pie));
        Ok(token)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 解封 Nole：建一枚**无载荷**的权柄载体 → 建门闩（全权）→ 返 token。
///
/// 与另两者的差别就是"没有第二步"：Pole 要分配物理帧并 auto-map 创建者视图，
/// Hole 要建槽（但**不预分配**——第一条消息由推者带进来）；Nole 建完 meta 就结束了
/// ——这正是"无数据面"的含义。
fn unseal_nole(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        // **铸币权收在 S 态**：这是一条**政策**，不是能力代数的推论——铃靠 `Accord`
        // 从父域流下去，不靠子域自铸。铸出来的那枚能转授给谁，仍由能力代数回答
        // （accord/narrow/revoke）。要加严或放开，动的就是这一行。
        if !ident.team.space.kind().is_supervisor() {
            return Err(GateError::Denied);
        }
        let task = current().running_task().ok_or(GateError::Denied)?;
        let meta = mail::nole::NoleMeta::new(task.ident.id);
        let pie: Pie<mail::nole::NoleMeta> = gate::new_pie(
            meta,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        // 同上：紧贴 push。
        let mut pies = task.pies.lock();
        pies.try_reserve(1).map_err(|_| GateError::OoM)?;
        pies.push(AnyPie::Nole(pie));
        Ok(token)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 解封 Pole：建实体 → 建门闩 → **auto-map 创建者视图**（创建者全权 → R|W）。
fn unseal_pole(frame: &mut TrapContext, size: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let meta = mail::pole::meta(size, task.ident.id)?;
        let task_space = task.ident.team.space.clone();
        let pie: Pie<mail::pole::PoleMeta> = gate::new_pie(
            meta.clone(),
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        );
        let token = pie.token;
        // 创建者自留 pie 全权 → 开闩走 R|W（U 位由空间策略决定）。
        // **预留贴在这里**：`open` 会落下创作者视图（映射），那才是不可撤回的一步；
        // 贴它之前而不是函数最前面，是因为前面的东西都能靠 drop 退回。
        task.pies
            .lock()
            .try_reserve(1)
            .map_err(|_| GateError::OoM)?;
        let creator_flags = task_space
            .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        mail::pole::open(&meta, token, &task_space, creator_flags)?;
        task.pies.lock().push(AnyPie::Pole(pie));
        Ok(token)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 开闩：借映 Pole 页进当前任务空间 → `(VA, 这一段多大)`（同 token 幂等复用）。
/// 仅对 Pole 成立。
fn open(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: usize) -> Outcome {
    let looked = current()
        .running_task()
        .ok_or(GateError::Denied)
        .and_then(|task| gate::accede(&task, token, Need::Fetch));
    let r = match looked.and_then(|p| usable(&p).map(|()| p)) {
        Err(e) => Err(e),
        Ok(AnyPie::Pole(p)) => match subset_to_pte(p.permission) {
            Err(e) => Err(e),
            Ok(flags) => mail::pole::open(
                p.meta(),
                token,
                &ident.team.space,
                ident.team.space.pte_policy(flags),
            ),
        },
        // Hole 没有「开闩」这回事：它的开闩就是 Push/Pull。
        Ok(AnyPie::Hole(_)) => Err(GateError::Denied),
        // Nole 更没有：它没有载荷可以借映进任何空间。
        Ok(AnyPie::Nole(_)) => Err(GateError::Denied),
        // Tole 也没有：它只有一张「我记着哪几枚孔」的格子表。
        Ok(AnyPie::Tole(_)) => Err(GateError::Denied),
    };
    answer_pair(frame, r);
    Outcome::Resume
}

/// 关闩：撤该 token 的映射（幂等）。仅对 Pole 成立。
///
/// **不过存活闸**：撤的是**调用方自己那张 PTE**，资源已封印也得撤得掉——否则
/// "封印后借入映射撤不掉"。故这里不走 [`gate::accede`]
/// （它含存活闸），只 `locate` + 判权 + 判「被关住」；`pole::shut` 里同样没有存活闸，
/// 两处是同一条语义，与 `Release`「你总得能放下手里的东西」对齐。权限要 `R`。
fn shut(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: usize) -> Outcome {
    let _ = &ident;
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let pie = gate::locate(&task, token)?;
        if !pie.allows(Need::Fetch) {
            return Err(GateError::Denied);
        }
        usable(&pie)?;
        match pie {
            AnyPie::Pole(p) => mail::pole::shut(p.meta(), token).map(|()| 0),
            // Hole / Nole / Tole 都没有「关闩」这回事（无映射可撤）。
            AnyPie::Hole(_) | AnyPie::Nole(_) | AnyPie::Tole(_) => Err(GateError::Denied),
        }
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 封印资源（generic）：**只有资源开辟者**可做（`owner` 是 Meta 字段，O(1) 判定）。
///
/// 只置死 + 唤醒等待者，**不摘表项**——持有者仍须 `Release` 收尾。故 `Release`
/// 与 `Shut` 是**仅有的两处**不过存活闸的操作（理由各异：否则封印即泄漏表项 /
/// 封印之后自己那张 PTE 撤不掉）。
fn seal(frame: &mut TrapContext, token: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let me = current().running_task().ok_or(GateError::Denied)?;
        // `locate` 只定位；**死活先判、再判"是不是我开的"**——与 `accede` 同一条顺序，
        // 判据本身不同（`Seal` 要的是 `owner`，不是某一位权）。
        let pie = gate::locate(&me, token)?;
        if !pie.alive() {
            return Err(GateError::Dead);
        }
        if pie.owner() != Some(me.ident.id) {
            return Err(GateError::Denied);
        }
        match &pie {
            AnyPie::Hole(h) => mail::hole::seal(h.meta()),
            AnyPie::Pole(pl) => mail::pole::seal(pl.meta()),
            AnyPie::Nole(v) => mail::nole::seal(v.meta()),
            AnyPie::Tole(t) => mail::tole::seal(t.meta()),
        }
        Ok(0)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 转授 / 交出一枚给另一个任务 → **对端侧**那枚的 token（撤回句柄）。
///
/// 四道闸（在表内 / 存活 / 持 `VEST` / 覆盖子集 / 未被关住）已下沉到核心
/// （`gate::accord`）——"交出"要在调用方表内就地写锚，抄件做不到。本层只补两件核心
/// 做不到的事：**① 过「被关住」闸**（陈旧锚在此自愈；核心不依赖 scheduler，核不了），
/// **② 把目标解析成 `Weak`**。
fn accord(frame: &mut TrapContext, src_token: usize, dst_id: usize, subset: Permission) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let caller = current().running_task().ok_or(GateError::Denied)?;
        // 源枚**只定位**：存活/持 `VEST`/覆盖子集/形态一致这四道闸在 `gate::accord` 里
        // （"交出"要在调用方表内就地写锚，抄件做不到）。
        let src = gate::locate(&caller, src_token)?;
        usable(&src)?;
        let dst = muster(dst_id).ok_or(GateError::Denied)?;
        gate::accord(&caller, src_token, &dst, subset)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 收窄本 pie 权限（就地改写，单调）：Pole 同步降页表段权限。
///
/// **死活先于覆盖子集**（与 [`gate::accede`] 同一条顺序）：已封印的 token 一律答 `Dead`，
/// 不因"子集越权"这个恰好排在更前的判据换成 `Denied`——同一个 token 的答案不该按动词变。
/// `Narrow` 本身不要求任何权利位（它只会把权限收小），故这里不用 `accede`，只
/// `locate` + 判死活。
fn narrow(frame: &mut TrapContext, token: usize, subset: Permission) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let pie = gate::locate(&task, token)?;
        if !pie.alive() {
            return Err(GateError::Dead);
        }
        if !pie.covers(subset) {
            return Err(GateError::Denied);
        }
        // Pole 的第二步（降页表段权限）要在改写权限位**之前**成功；其余三种只有一步。
        let pole_meta = match &pie {
            AnyPie::Pole(p) => Some(p.meta().clone()),
            AnyPie::Hole(_) | AnyPie::Nole(_) | AnyPie::Tole(_) => None,
        };
        if let Some(meta) = pole_meta {
            mail::pole::narrow(&meta, token, subset_to_pte(subset)?)?;
        }
        let mut pies = task.pies.lock();
        match pies.iter_mut().find(|p| p.token() == token) {
            Some(p) => gate::narrow(p, subset).map(|()| 0),
            None => Err(GateError::Denied),
        }
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 收回授与 `dst` 的副本（含其全部后代，幂等）。
///
/// `token` = 该副本在**对端表里**的句柄（`Accord` 的返回值，经线形送达）——
/// 不是我这边的 token。鉴权 = 「这枚的 `sire` 在我表里」。
fn revoke(frame: &mut TrapContext, dst_id: usize, token: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let caller = current().running_task().ok_or(GateError::Denied)?;
        let target = muster(dst_id).ok_or(GateError::Denied)?;
        gate::revoke(&caller, &target, token, &gate::snap()).map(|_| 0)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 收拢：报出本任务权限表第 `index` 份——**唯一的枚举手段**（`protocol::startup::moor()`
/// 靠它发现「父域授给我的那枚门闩」这类未知句柄）。已知句柄求事实用 `Reserve`。
///
/// 越界 → 全哨兵（token 0 / 权限空 / vestor 0），**不报错**。
/// a0 = token；a1 = 权限位（低 32）| vestor task id（高 32）。
/// 锁序：先克隆出 pie（还 pies 锁），再取快照求 vestor——两者同为 L3，绝不嵌套。
fn collect(frame: &mut TrapContext, index: usize) -> Outcome {
    let pie = current()
        .running_task()
        .and_then(|t| t.pies.lock().get(index).cloned());
    let (token, perm_bits, vestor_id) = match pie {
        Some(p) => (
            p.token(),
            p.permission().bits() as usize,
            gate::vestor(&p, &gate::snap()).unwrap_or(0),
        ),
        None => (0, 0, 0),
    };
    frame.gpr.set_x(Gprs::A0, token);
    frame
        .gpr
        .set_x(Gprs::A1, (vestor_id << 32) | (perm_bits & 0xffff_ffff));
    Outcome::Resume
}

/// 查这枚门闩的来历：`vestor`（谁授的，转手即改写）+ `owner`（资源谁开的，任何
/// 副本共享同一事实）+ **记号**（拷进调用方那段缓冲）。**不过存活闸**：`owner` 随资源
/// 不变，封印不使它消失。
///
/// a0 = vestor（无 → 0），a1 = 记号长度（高 32）| owner（低 32）；记号的内容经
/// `buf`/`cap` 那一段拷出，**只拷内容那一段**（尾随填充不上线）。
///
/// 契约同 `MailCall::Pull` 的"**要么全取、要么一个字节都不动**"：装不下（长度 > `cap`）
/// 先拒，`copy_out` 自己也是整段验完才写。表里无此 token → `Denied`。
///
/// **记号只长在孔上**：别的资源（Pole/Nole/Tole）问不到它 ⇒ `Denied`——问不到就是
/// "这一条候选不成立"，不假装有一格空名。拷出在 `find` 放锁之后（它要过
/// `space.segments` = L2，持 L3 锁做反向嵌套会当场 panic）。
fn reserve(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    token: usize,
    buf: usize,
    cap: usize,
) -> Outcome {
    let r = (|| -> Result<(usize, usize, usize), GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        // **只定位**：`owner` 是资源来历，封印不使它消失（见函数头注）——这里刻意不过
        // 死活闸，`owner()` 自己用 `alive()` 把"已封印 ⇒ `Dead`"答出来。
        let p = gate::locate(&task, token)?;
        let owner = p.owner().ok_or(GateError::Dead)?;
        let AnyPie::Hole(h) = &p else {
            return Err(GateError::Denied);
        };
        let mark = h.meta().mark();
        let len = mark.len();
        if len > cap {
            return Err(GateError::Denied);
        }
        if !mail::copy_out(&ident.team.space, mark.text(), buf) {
            return Err(GateError::Denied);
        }
        Ok((gate::vestor(&p, &gate::snap()).unwrap_or(0), owner, len))
    })();
    match r {
        Ok((vestor_id, owner_id, len)) => {
            frame.gpr.set_x(Gprs::A0, vestor_id);
            // 打包见 `env::wire::frompair` 的 `(TaskId, TaskId, usize)`：第三格进高半。
            frame
                .gpr
                .set_x(Gprs::A1, (len << 32) | (owner_id & 0xffff_ffff));
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// 自释：放下我持有的一份（含其全部后代；Pole 同步撤映射）。
///
/// **不判存活**——与 `shut` 并列，是**仅有的两处例外**：`Seal` 不摘表项，若本操作
/// 也判存活，封印后的表项就永远摘不掉。语义 =「你总得能放下手里的东西」。不需要
/// 任何权限位。
fn release(frame: &mut TrapContext, token: usize) -> Outcome {
    let r = match current().running_task() {
        Some(task) => gate::release(&task, token, &gate::snap()).map(|_| 0),
        None => Err(GateError::Denied),
    };
    answer(frame, r);
    Outcome::Resume
}
