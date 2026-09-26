//! 多路等待轴（class 9 `ToleCall`）—— 一枚"组"的四件事：造、挂、摘、等。
//!
//! 与权柄轴（class 7 `PieCall`）的分界：本轴**不搬许可的生死**，只改"这一组我关心
//! 哪几枚可等地"；与数据轴（class 5 `MailCall`）的分界：本轴**不搬载荷**。
//!
//! **成员只有两种**：孔（一个方向）与铃——两者都有"有一位可读的就绪谓词"。故挂/摘
//! 收两枚 token（组 + 成员），等只要组那一枚。
//!
//! **token → 表内解析在这里**（数据面 `mail::tole` 不碰任务表），分三支：
//! `resolve`（表里取一枚门闩：表内 → 权限 → 存活）、`rack`（认成组）、
//! `mate`（认成成员：类型 + 方向归一 + 寿命边，**一处产出那一对**，不会错配）。
//!
//! **「被关住」的挂点按资源语义判**：`Await` 是"用等待位" ⇒ 查**组**门闩；`Attach` 是
//! "我要用这枚成员" ⇒ 查**成员**门闩；`Detach` 是收场 ⇒ 不查（不查才清得掉）。
//!
//! `Await` 是**唯一可能换帧**的动词（其余三个都不挂起）。唤醒只是提示：挂起过一侧
//! 返回恒是预置值（`PieToken::NONE`），调用方按 deadline 循环、醒来自己按组快照复核。

use alloc::sync::{Arc, Weak};

use env::{HoleDir, ToleCall};

use env::{PieToken, ToleFail, Wait};

use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail::tole::Mate;
use crate::work::mail::{ToleMeta, tole};
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, AnyPie, Need, Permission, Pie};
use crate::work::unit::life::Life;
use crate::work::unit::task::TaskIdent;

use super::mail::Outcome;
use super::pie::usable;

/// 本轴的四个动词。返回 `None` = 本次调用不属于本轴（交还门面继续匹配）。
pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: ToleCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    let _ = &ident;
    Some(match call {
        ToleCall::Unseal { shared } => unseal(frame, shared),
        ToleCall::Attach { tole, pie, dir } => attach(frame, tole, pie, dir),
        ToleCall::Detach { tole, pie, dir } => detach(frame, tole, pie, dir),
        // 唯一可能换帧的一支：不走 `Outcome::Resume` 的统一出口。
        ToleCall::Await { tole, millis } => return Some(await_(frame, tole, millis)),
    })
}

/// 造一个空组：建 meta → 建门闩（`FETCH | STORE | VEST`，独占时再加 `ONLY`）→ 返 token。
///
/// **种类只在创建点定**（`shared` 就是从这一枚位推出来的），之后不可变：`narrow` 不得撤
/// `ONLY` ⇒ 独占组变不成共享组。
///
/// - **独占组**（`shared = false`）：门闩带 `ONLY` ⇒ 只有一条等待位（组键一次只兑现一个
///   等待者），故这枚资源只允许一个使用者——授出即**移交**（`Accord` 校验、写锚），复制
///   不出来；
/// - **共享组**（`shared = true`）：门闩**不带** `ONLY` ⇒ 可 `Accord` 复制给多个任务、
///   没有锚（等待权不会"被关住"）；多个持有者可以同时等同一只组键，故那一路的唤醒是
///   **提示型**（整链放行，人人自取快照复核，见 `messenger::knock`）。
///
/// 两者都带 `VEST`：共享组若不可复制，"多个使用者"就是一句空话。
///
/// **不在 S 态设闸**（与 `UnsealNole` 的铸币权政策不同）：组不授予任何对资源的权柄，
/// 它只是"我自己关心哪几枚可等地"的账。U 态域等多个源是常态，不该逼它回 S 态。
fn unseal(frame: &mut TrapContext, shared: bool) -> Outcome {
    let r = (|| -> Result<usize, ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let meta = tole::meta(task.ident.id);
        // 读写两支人人有；`VEST` 两种都有（共享组靠它复制出去）；`ONLY` 只给独占组。
        let mut latch = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            latch |= Permission::ONLY;
        }
        let pie: Pie<ToleMeta> = gate::new_pie(meta, latch, None);
        let token = pie.token;
        // **紧贴 push**：`pie` 是最后一步造的，drop 它即回收资源实体 ⇒ 失败就地退回。
        let mut pies = task.pies.lock();
        pies.try_reserve(1).map_err(|_| ToleFail::OoM)?;
        pies.push(AnyPie::Tole(pie));
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

/// 挂一格：组要 `STORE`（改我自己的账），成员要 `FETCH`（"我要用它"）。
///
/// **成员的「被关住」在这里查**：挂一格就是声明"我要用它"——它若已被我交出去
/// （`HandedOver`），挂进来只会让组替我答"它有事"而我又取不走它。组的「被关住」不查：
/// `ONLY` 只在**等待位**上成立，改池子不算用它（见模块头）。
fn attach(frame: &mut TrapContext, group: PieToken, member: PieToken, dir: HoleDir) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        let latch = gate::accede::<ToleFail>(&task, member, Need::Fetch)?;
        usable::<ToleFail>(&latch)?;
        let (mate, life) = mate(&latch, dir)?;
        tole::attach(&meta, mate, life)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

/// 摘一格：组要 `STORE`，成员要 `FETCH`；**两道「被关住」都不查**（收场动作）。
fn detach(frame: &mut TrapContext, group: PieToken, member: PieToken, dir: HoleDir) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        let latch = gate::accede::<ToleFail>(&task, member, Need::Fetch)?;
        let (mate, _life) = mate(&latch, dir)?;
        tole::detach(&meta, mate)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

/// 等到组里任意一格有事：先查（当场答），再挂。
///
/// 「先查」不可省：**已就绪**的那一格若只靠投信唤醒，而它在我们登记之前就已经就绪，
/// 这一觉就睡到期限了（与 `hole::wait` 的"先探"同一条理由）。
///
/// **组的「被关住」在这里查**：等待位是 `ONLY` 保护的那一件事——我把等待权交出去了，
/// 就轮到对方等，我在交出期间不问。
///
/// **解析失败三种各有其名**（不折平）：表里没有 / 权不够 → `Denied`；已封印 → `Dead`；
/// 等待权已被我过户（`usable`）→ `HandedOver`。折成一个码会把"组没了，换策略"与"号拿错了，
/// 修 bug"压成同一件——而这两件事的处置正好相反。顺序本身由共用的 `gate::accede` 决定
/// （死活先于权限），本处只补第三维。
fn await_(frame: &mut TrapContext, group: PieToken, millis: Wait) -> Outcome {
    let dur = millis.into_duration();
    // 当前任务那份 `Arc` 是**临时量**：第一段闭包里就落地（跨挂起不得持强引用）。
    let looked = current()
        .running_task()
        .ok_or(ToleFail::Denied)
        .and_then(|task| gate::accede::<ToleFail>(&task, group, Need::Fetch))
        .and_then(|latch| {
            usable::<ToleFail>(&latch)?;
            rack(&latch)
        });
    let meta = match looked {
        Ok(meta) => meta,
        Err(e) => {
            answer_void(frame, Err(e));
            return Outcome::Resume;
        }
    };
    if let Some((token, dir)) = ready(&meta) {
        answer_pair(frame, token, dir);
        return Outcome::Resume;
    }
    // 挂起后恢复读到的 a0/a1 = 挂起前预置值 ⇒ 预置「没等到」；当场判定再改写。
    answer_pair(frame, PieToken::NONE, HoleDir::Pull);
    match tole::wait(&meta, dur) {
        // 未换帧：信标已至（组变过）或期限为零 ⇒ 以当前快照为准再查一遍。
        Ok(Handoff::Resume(())) => {
            if let Some((token, dir)) = ready(&meta) {
                answer_pair(frame, token, dir);
            }
            Outcome::Resume
        }
        Ok(Handoff::Switch(pa)) => Outcome::Park(pa as *mut TrapContext),
        Err(e) => {
            answer_void(frame, Err(e));
            Outcome::Resume
        }
    }
}

/// 快照里**现在就有一格可用**的那一格——答出"是哪一枚"（本端表里的号）+ 方向。
///
/// 只有本端**表里还有**那一枚成员时才答得出号：格子记的是资源身份，号是表里的东西。
/// 表里找不到（自己已经把它放下了）⇒ 跳过那一格。
///
/// 两种成员各读各的位：孔按方向读槽（空/满），铃读那一位（响/没响）；铃没有方向，
/// 答出的方向恒 `Pull`（与 `MailCall::Wait` 只认 `Pull` 同一条约定）。
fn ready(meta: &ToleMeta) -> Option<(PieToken, HoleDir)> {
    let task = current().running_task()?;
    let cells = meta.cells();
    let pies = task.pies.lock();
    for cell in cells {
        match cell.mate() {
            Mate::Hole(id, dir) => {
                let Some(pie) = pies
                    .iter()
                    .find(|p| matches!(p, AnyPie::Hole(h) if h.meta().id() == id))
                else {
                    continue;
                };
                let AnyPie::Hole(h) = pie else { continue };
                if h.meta().ready(dir) {
                    return Some((pie.token(), dir));
                }
            }
            Mate::Nole(id) => {
                let Some(pie) = pies
                    .iter()
                    .find(|p| matches!(p, AnyPie::Nole(n) if n.meta().id() == id))
                else {
                    continue;
                };
                let AnyPie::Nole(n) = pie else { continue };
                if n.meta().ready() {
                    return Some((pie.token(), HoleDir::Pull));
                }
            }
        }
    }
    None
}

// 按 token 取用（表内 → 死活 → 权限）已下沉到核心 `gate::accede`，本轴不再自己写一份：
// 那一份曾经把"权限"排在"死活"之前，于是同一个已封印的组/成员在两个轴上答不同的码。
// 本层只剩三件事：把当前任务递进去、`usable`（"被关住"第三维，必须在放锁后判）、
// `rack`/`mate`（认成组、认成成员）。

/// 认成一枚架子（组）：不是组 ⇒ `Denied`。
fn rack(pie: &AnyPie) -> Result<Arc<ToleMeta>, ToleFail> {
    match pie {
        AnyPie::Tole(t) => Ok(t.meta().clone()),
        // 孔 / 页 / 铃都不是组：这一轴只有组是架子。
        _ => Err(ToleFail::Denied),
    }
}

/// 认成一格成员：**类型 + 方向归一 + 寿命边，一处产出那一对**（不会错配）。
///
/// 成员只有孔与铃：孔自带方向；**铃只有一条方向**，故它只认 `Pull`（别的值不是
/// "暂时没有"，是不存在这个操作——同 `MailCall::Wait`）。
fn mate(pie: &AnyPie, dir: HoleDir) -> Result<(Mate, Weak<Life>), ToleFail> {
    match pie {
        AnyPie::Hole(h) => Ok((Mate::Hole(h.meta().id(), dir), h.meta().life())),
        AnyPie::Nole(n) if dir == HoleDir::Pull => Ok((Mate::Nole(n.meta().id()), n.meta().life())),
        _ => Err(ToleFail::Denied),
    }
}

/// 写回 a0（单值返回；错误路径只写 a0）。
fn answer(frame: &mut TrapContext, r: Result<usize, ToleFail>) {
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(v) => v,
            Err(e) => e.code() as usize,
        },
    );
}

/// 写回 a0（`()` 返回：成功写 0，失败写负码）。
fn answer_void(frame: &mut TrapContext, r: Result<(), ToleFail>) {
    answer(frame, r.map(|()| 0));
}

/// 写回 a0 + a1（两件返回；契约见 `crates/env` 的 `FromPair`）。
fn answer_pair(frame: &mut TrapContext, token: PieToken, dir: HoleDir) {
    frame.gpr.set_x(Gprs::A0, token.get());
    frame.gpr.set_x(
        Gprs::A1,
        match dir {
            HoleDir::Pull => 0,
            HoleDir::Push => 1,
        },
    );
}
