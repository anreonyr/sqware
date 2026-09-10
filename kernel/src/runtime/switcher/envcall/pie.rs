//! envcall 权柄轴（class 7 `PieCall`）—— 许可的生死与流动，十一个操作。
//!
//! 与数据轴（`envcall/mail.rs`）的分界：本模块**不搬运载荷**——传的是许可，
//! 内容走 class 5。三条轴（资源 / 持有 / 转授）见 `crates/env/src/fid.rs`。
//!
//! # 两个查询原语
//!
//! 十一个操作里有六个在开头做同一件事：**按 token 在我表里取得，顺带判存活与判权**。
//! 那件事在此立为 [`resolve`]（判权）与 [`find`]（只判存在）——不是抄六遍：
//! 逐处手写曾导致判定顺序不一（`allows` 先或 `alive` 先），**同一个已封印的 token
//! 按调用的动词不同报出 `Denied` 或 `Dead` 两个答案**。现在顺序只有一处：
//! 表里没有 → `Denied`；已封印 → `Dead`；权不够 → `Denied`。
//!
//! 显式不过闸的两个：`Release`（自释必须能在封印后收尾，否则表项永远摘不掉）、
//! `Reserve`（`owner` 是资源来历，封印不使它消失）——它们走 [`find`]。
//!
//! `ident` 的所有权移交与门面一致；本轴的操作都不换帧（无挂起）。

use alloc::sync::Arc;

use env::PieCall;

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::{self, AnyPie, GateError, Need, Permission, Pie};
use crate::work::unit::task::TaskIdent;

use super::subset_to_pte;

/// 一次权柄 envcall 的落点：本轴从不换帧，故只有一个变体。
///
/// 与数据轴的 `Outcome` 同名同形——门面按 `Park` 的有无便可判定「本操作是否可能
/// 换帧」。本轴**没有** `Park` 分支，这不是遗漏：权柄操作不挂起。
pub(crate) enum Outcome {
    /// 续跑当前任务，返回值已写入 a0/a1。
    Resume,
}

/// 权柄轴的十一个操作。返回 `None` = 本次调用不属于本轴（交还门面继续匹配）。
pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: PieCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    let _ = &ident;
    Some(match call {
        PieCall::UnsealHole { mtu } => unseal_hole(frame, mtu),
        PieCall::UnsealPole { bytes } => unseal_pole(frame, bytes),
        PieCall::Open { token } => open(frame, ident, token.get()),
        PieCall::Shut { token } => shut(frame, ident, token.get()),
        PieCall::Seal { token } => seal(frame, token.get()),
        PieCall::Accord { src, dst, subset } => accord(frame, src.get(), dst.get(), subset),
        PieCall::Narrow { token, subset } => narrow(frame, token.get(), subset),
        PieCall::Revoke { dst, token } => revoke(frame, dst.get(), token.get()),
        PieCall::Collect { index } => collect(frame, index),
        PieCall::Reserve { token } => reserve(frame, token.get()),
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

/// 按 token 在**当前任务**表里取得，判存活、判权。
///
/// 三种失败各有其名：表里没有 → `Denied`（不是我的）；已封印 → `Dead`；
/// 权不够 → `Denied`。克隆出表后再还锁——`Pie` 必须在锁外 drop（最后一份 drop
/// 会跑 `Meta::drop`，唤醒等待者/撤映射/还帧全是 L3 或更外层的活）。
fn resolve(token: usize, need: Need) -> Result<AnyPie, GateError> {
    let pie = find(token)?;
    if !pie.alive() {
        return Err(GateError::Dead);
    }
    if !pie.allows(need) {
        return Err(GateError::Denied);
    }
    Ok(pie)
}

/// 只判「这枚在我表里吗」——不过存活闸。
///
/// 给 `Release`（封印后仍须能收尾）与 `Reserve`（`owner` 随资源不变）用。
fn find(token: usize) -> Result<AnyPie, GateError> {
    let task = current().running_task().ok_or(GateError::Denied)?;
    let pies = task.pies.lock();
    pies.iter()
        .find(|p| p.token() == token)
        .cloned()
        .ok_or(GateError::Denied)
}

/// 解封 Hole：建资源实体 → 建门闩（原始自持：无 sire）→ 落表。
///
/// 门闩持资源实体的强引用——**寿命即能力寿命**：最后一份消失时资源随之回收。
fn unseal_hole(frame: &mut TrapContext, mtu: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let meta = mail::hole::meta(mtu, task.ident.id)?;
        let pie: Pie<mail::hole::HoleMeta> = gate::new_pie(
            meta,
            Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
            None,
        );
        let token = pie.token;
        task.pies.lock().push(AnyPie::Hole(pie));
        Ok(token)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 解封 Pole：建实体 → 建门闩 → **auto-map 创建者视图**（创建者全权 → R|W）。
fn unseal_pole(frame: &mut TrapContext, bytes: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let meta = mail::pole::meta(bytes, task.ident.id)?;
        let task_space = task.ident.team.space.clone();
        let pie: Pie<mail::pole::PoleMeta> = gate::new_pie(
            meta.clone(),
            Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
            None,
        );
        let token = pie.token;
        // 创建者自留 pie 全权 → 开闩走 R|W（U 位由空间策略决定）。
        let creator_flags = task_space
            .pte_policy(PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D);
        mail::pole::open(&meta, token, &task_space, creator_flags)?;
        task.pies.lock().push(AnyPie::Pole(pie));
        Ok(token)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 开闩：借映 Pole 页进当前任务空间 → VA（同 token 幂等复用）。仅对 Pole 成立。
fn open(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: usize) -> Outcome {
    let r = match resolve(token, Need::Read) {
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
    };
    answer(frame, r);
    Outcome::Resume
}

/// 关闩：撤该 token 的映射（幂等）。仅对 Pole 成立。
fn shut(frame: &mut TrapContext, ident: Arc<TaskIdent>, token: usize) -> Outcome {
    let _ = &ident;
    let r = match resolve(token, Need::Read) {
        Err(e) => Err(e),
        Ok(AnyPie::Pole(p)) => mail::pole::shut(p.meta(), token).map(|()| 0),
        Ok(AnyPie::Hole(_)) => Err(GateError::Denied),
    };
    answer(frame, r);
    Outcome::Resume
}

/// 封印资源（generic）：**只有资源开辟者**可做（`owner` 是 Meta 字段，O(1) 判定）。
///
/// 只置死 + 唤醒等待者，**不摘表项**——持有者仍须 `Release` 收尾。故 `Release`
/// 是唯一不过存活闸的操作（否则封印即泄漏表项）。
fn seal(frame: &mut TrapContext, token: usize) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let me = current().running_task().ok_or(GateError::Denied)?;
        let pie = {
            let pies = me.pies.lock();
            pies.iter().find(|p| p.token() == token).cloned()
        }
        .ok_or(GateError::Denied)?;
        if !pie.alive() {
            return Err(GateError::Dead);
        }
        if pie.owner() != Some(me.ident.id) {
            return Err(GateError::Denied);
        }
        match &pie {
            AnyPie::Hole(h) => mail::hole::seal(h.meta()),
            AnyPie::Pole(pl) => mail::pole::seal(pl.meta()),
        }
        Ok(0)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 转授子集给另一个任务 → 对端侧那枚的 token（撤销句柄）。
///
/// 四道闸：存活 → 有转授权（VEST 或 BACK）→ `subset` 非空且 ⊆ 当前权限 →
/// BACK 守门（带 BACK 只能授回 sire 的持有者；原始自持不受限）。
fn accord(frame: &mut TrapContext, src_token: usize, dst_id: usize, subset: Permission) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let src = find(src_token)?;
        if !src.alive() {
            return Err(GateError::Dead);
        }
        if !src.allows(Need::Grant) {
            return Err(GateError::Denied);
        }
        // subset 已由 Wire 校验式 unpack（非法位 → Err），此处只查非空 & ⊆。
        if !src.covers(subset) {
            return Err(GateError::Denied);
        }
        if !gate::vestable(&src, dst_id, &gate::snap()) {
            return Err(GateError::Denied);
        }
        let target = muster(dst_id).ok_or(GateError::Denied)?;
        gate::accord(&src, &target, subset)
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 收窄本 pie 权限（就地改写，单调）：Pole 同步降页表段权限。
fn narrow(frame: &mut TrapContext, token: usize, subset: Permission) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let pole_meta = {
            let pies = task.pies.lock();
            let pie = pies
                .iter()
                .find(|p| p.token() == token)
                .ok_or(GateError::Denied)?;
            if !pie.covers(subset) {
                return Err(GateError::Denied);
            }
            match pie {
                AnyPie::Hole(p) => {
                    if !p.meta().alive() {
                        return Err(GateError::Dead);
                    }
                    None
                }
                AnyPie::Pole(p) => Some(p.meta().clone()),
            }
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

/// 收拢：报出本任务权限表第 `index` 份——**唯一的枚举手段**（`handshake::moor()`
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
/// 副本共享同一事实）。**不过存活闸**：`owner` 随资源不变，封印不使它消失。
///
/// a0 = vestor（无 → 0），a1 = owner。表里无此 token → `Denied`。
fn reserve(frame: &mut TrapContext, token: usize) -> Outcome {
    let r = (|| -> Result<(usize, usize), GateError> {
        let p = find(token)?;
        let owner = p.owner().ok_or(GateError::Dead)?;
        Ok((gate::vestor(&p, &gate::snap()).unwrap_or(0), owner))
    })();
    match r {
        Ok((vestor_id, owner_id)) => {
            frame.gpr.set_x(Gprs::A0, vestor_id);
            frame.gpr.set_x(Gprs::A1, owner_id);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// 自释：放下我持有的一份（含其全部后代；Pole 同步撤映射）。
///
/// **唯一不判存活的操作**：`Seal` 不摘表项，若本操作也判存活，封印后的表项就
/// 永远摘不掉。语义 =「你总得能放下手里的东西」。不需要任何权限位。
fn release(frame: &mut TrapContext, token: usize) -> Outcome {
    let r = match current().running_task() {
        Some(task) => gate::release(&task, token, &gate::snap()).map(|_| 0),
        None => Err(GateError::Denied),
    };
    answer(frame, r);
    Outcome::Resume
}
