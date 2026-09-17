//! 多路等待轴（class 9 `ToleCall`）—— 一枚"组"的四件事：造、挂、摘、等。
//!
//! 与权柄轴（class 7 `PieCall`）的分界：本轴**不搬许可的生死**，只改"这一组我关心
//! 哪几枚孔"；与数据轴（class 5 `MailCall`）的分界：本轴**不搬载荷**。
//!
//! **token → 表内解析留在这里**（数据面 `mail::tole` 不碰任务表）：挂/摘要两枚 token
//! （组 + 孔），等只要组那一枚。判据顺序与权柄轴同款——表里没有 → `Denied`；
//! 已封印 → `Dead`；权不够 → `Denied`。
//!
//! `Await` 是**唯一可能换帧**的动词（其余三个都不挂起）。唤醒只是提示：挂起过一侧
//! 返回恒是预置值（`PieToken::NONE`），调用方按 deadline 循环、醒来自己按组快照复核。

use alloc::sync::Arc;
use core::time::Duration;

use env::{HoleDir, ToleCall};

use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail::{HoleMeta, ToleMeta, tole};
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, AnyPie, GateError, Need, Permission, Pie};
use crate::work::unit::task::TaskIdent;

use super::mail::Outcome;

/// 本轴的四个动词。返回 `None` = 本次调用不属于本轴（交还门面继续匹配）。
pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: ToleCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    let _ = &ident;
    Some(match call {
        ToleCall::Unseal => unseal(frame),
        ToleCall::Hang { tole, pie, dir } => hang(frame, tole.get(), pie.get(), dir),
        ToleCall::Unhang { tole, pie, dir } => unhang(frame, tole.get(), pie.get(), dir),
        // 唯一可能换帧的一支：不走 `Outcome::Resume` 的统一出口。
        ToleCall::Await { tole, millis } => return Some(await_(frame, tole.get(), millis)),
    })
}

/// 造一个空组：建 meta → 建门闩（全权）→ 返 token。
///
/// **不在 S 态设闸**（与 `UnsealNole` 的铸币权政策不同）：组不授予任何对资源的权柄，
/// 它只是"我自己关心哪几枚孔"的账。U 态域等多个源是常态，不该逼它回 S 态。
fn unseal(frame: &mut TrapContext) -> Outcome {
    let r = (|| -> Result<usize, GateError> {
        let task = current().running_task().ok_or(GateError::Denied)?;
        let meta = tole::meta(task.ident.id);
        let pie: Pie<ToleMeta> = gate::new_pie(
            meta,
            Permission::READ | Permission::WRITE | Permission::VEST | Permission::CAGE,
            None,
        );
        let token = pie.token;
        // **紧贴 push**：`pie` 是最后一步造的，drop 它即回收资源实体 ⇒ 失败就地退回。
        let mut pies = task.pies.lock();
        pies.try_reserve(1).map_err(|_| GateError::OoM)?;
        pies.push(AnyPie::Tole(pie));
        Ok(token)
    })();
    answer(frame, r);
    Outcome::Resume
}

fn hang(frame: &mut TrapContext, rack: usize, target: usize, dir: HoleDir) -> Outcome {
    let r = (|| -> Result<(), GateError> {
        let meta = tole_of(rack, Need::Write)?;
        let hole = hole_of(target, Need::Read)?;
        tole::hang(&meta, &hole, dir)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

fn unhang(frame: &mut TrapContext, rack: usize, target: usize, dir: HoleDir) -> Outcome {
    let r = (|| -> Result<(), GateError> {
        let meta = tole_of(rack, Need::Write)?;
        let hole = hole_of(target, Need::Read)?;
        tole::unhang(&meta, &hole, dir)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

/// 等到组里任意一格有事：先查（当场答），再挂。
///
/// 「先查」不可省：**已就绪**的那一格若只靠投信唤醒，而它在我们登记之前就已经就绪，
/// 这一觉就睡到期限了（与 `hole::wait` 的"先探"同一条理由）。
fn await_(frame: &mut TrapContext, rack: usize, millis: usize) -> Outcome {
    let dur = if millis == usize::MAX {
        Duration::MAX
    } else {
        Duration::from_millis(millis as u64)
    };
    let Ok(meta) = tole_of(rack, Need::Read) else {
        answer_void(frame, Err(GateError::Denied));
        return Outcome::Resume;
    };
    if let Some((token, dir)) = ready(&meta) {
        answer_pair(frame, token, dir);
        return Outcome::Resume;
    }
    // 挂起后恢复读到的 a0/a1 = 挂起前预置值 ⇒ 预置「没等到」；当场判定再改写。
    answer_pair(frame, 0, HoleDir::Pull);
    match tole::wait_tole(&meta, dur) {
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
/// 只有本端**表里还有**那一枚孔时才答得出号：格子记的是资源身份，号是表里的东西。
/// 表里找不到（自己已经把孔放下了）⇒ 跳过那一格。
fn ready(meta: &ToleMeta) -> Option<(usize, HoleDir)> {
    let task = current().running_task()?;
    let cells = meta.cells();
    let pies = task.pies.lock();
    for cell in cells {
        let Some(pie) = pies.iter().find(|p| match p {
            AnyPie::Hole(h) => h.meta().id() == cell.hole(),
            _ => false,
        }) else {
            continue;
        };
        let AnyPie::Hole(h) = pie else { continue };
        if h.meta().ready(cell.dir()) {
            return Some((pie.token(), cell.dir()));
        }
    }
    None
}

/// 按 token 在**当前任务**表里取得（只判存在，与权柄轴的分工一致）。
fn any(token: usize) -> Result<AnyPie, GateError> {
    let task = current().running_task().ok_or(GateError::Denied)?;
    let pies = task.pies.lock();
    pies.iter()
        .find(|p| p.token() == token)
        .cloned()
        .ok_or(GateError::Denied)
}

/// 取得一枚组：不在表里 → `Denied`；已封印 → `Dead`；权不够 → `Denied`；不是组 → `Denied`。
fn tole_of(token: usize, need: Need) -> Result<Arc<ToleMeta>, GateError> {
    let pie = any(token)?;
    if !pie.alive() {
        return Err(GateError::Dead);
    }
    if !pie.allows(need) {
        return Err(GateError::Denied);
    }
    match pie {
        AnyPie::Tole(t) => Ok(t.meta().clone()),
        _ => Err(GateError::Denied),
    }
}

/// 取得一枚孔（挂进组的只能是孔：别的资源没有"有事"这回事）。
fn hole_of(token: usize, need: Need) -> Result<Arc<HoleMeta>, GateError> {
    let pie = any(token)?;
    if !pie.alive() {
        return Err(GateError::Dead);
    }
    if !pie.allows(need) {
        return Err(GateError::Denied);
    }
    match pie {
        AnyPie::Hole(h) => Ok(h.meta().clone()),
        _ => Err(GateError::Denied),
    }
}

/// 写回 a0（单值返回；错误路径只写 a0）。
fn answer(frame: &mut TrapContext, r: Result<usize, GateError>) {
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(v) => v,
            Err(e) => e.code() as usize,
        },
    );
}

/// 写回 a0（`()` 返回：成功写 0，失败写负码）。
fn answer_void(frame: &mut TrapContext, r: Result<(), GateError>) {
    answer(frame, r.map(|()| 0));
}

/// 写回 a0 + a1（两件返回；契约见 `crates/env` 的 `FromPair`）。
fn answer_pair(frame: &mut TrapContext, token: usize, dir: HoleDir) {
    frame.gpr.set_x(Gprs::A0, token);
    frame.gpr.set_x(
        Gprs::A1,
        match dir {
            HoleDir::Pull => 0,
            HoleDir::Push => 1,
        },
    );
}
