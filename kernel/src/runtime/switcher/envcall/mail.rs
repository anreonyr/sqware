//! envcall 数据轴（class 5 `MailCall`）—— 消息穿孔的三个操作。
//!
//! 与权柄轴（`envcall/pie.rs`）的分界：本模块**只搬运载荷**，不调用任何
//! 权柄函数（`accord`/`narrow`/`revoke`/`release`/`vestor`/`snap`）。两轴正交，
//! 判据见 `crates/env/src/fid.rs` 文件头。
//!
//! `ident` 的所有权移交与门面一致：可能触发 halt 的分支先 `drop(ident)`，否则
//! halt 时身份 Arc 仍持最后任务 team → space 不 drop，关机审计会误报帧泄漏。

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use env::{HoleDir, MailCall};

use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::trap::run;
use crate::work::unit::gate::{AnyPie, GateError, Need};
use crate::work::unit::task::TaskIdent;

/// 一次 envcall 的落点：续跑本任务，或换一帧跑（让出/挂起）。
///
/// 把「本操作是否可能换帧」写进类型，而不是留给调用方猜。
pub(crate) enum Outcome {
    /// 续跑当前任务：`frame` 即入参帧，返回值已写入 a0/a1。
    Resume,
    /// 换一帧跑：挂起/让出后交出的下一帧。
    Park(*mut TrapContext),
}

/// 数据轴的三个操作。返回 `None` = 本次调用不属于本轴（交还门面继续匹配）。
pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: MailCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    Some(match call {
        MailCall::Push { token, msg, len } => {
            push(frame, ident, token.get(), KVirt::from_raw(msg.get()), len)
        }
        MailCall::Pull { token, buf, max } => {
            pull(frame, ident, token.get(), KVirt::from_raw(buf.get()), max)
        }
        MailCall::Wait { token, dir, millis } => wait_dir(frame, ident, token.get(), dir, millis),
    })
}

/// 把一条消息推入 hole：权柄判定（W）→ 长度校验 → 锁外拷入堆暂存 → try_push。
///
/// `me` = 推者身份：内核盖章，与消息同槽交付收方（收方不必从报文里猜发送者）。
fn push(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: usize,
    msg: KVirt,
    len: usize,
) -> Outcome {
    let task = current().running_task();
    let me = task.as_ref().map(|t| t.ident.id).unwrap_or(0);
    let r = match task.and_then(|t| {
        let pies = t.pies.lock();
        let pie = pies.iter().find(|p| p.token() == token)?;
        if !pie.allows(Need::Write) {
            return Some(Err(GateError::Denied));
        }
        if !pie.alive() {
            return Some(Err(GateError::Dead));
        }
        match pie {
            AnyPie::Hole(p) => Some(Ok(p.meta().clone())),
            _ => None,
        }
    }) {
        Some(Ok(meta)) => {
            // 长度校验：必须 ≥1 且 ≤ hole.mtu（meta() 入口已校验 mtu∈[1,4096]）。
            if len == 0 || len > meta.mtu {
                Err(GateError::Denied)
            } else {
                // 锁外拷入堆暂存：slot = L3，Space.segments = L2，
                // 持 L3 调 L2 是 4→2 反向嵌套，禁止。
                let mut staging = alloc::vec![0u8; len];
                if mail::copy_in(&ident.team.space, &mut staging, msg.as_usize()) {
                    mail::hole::try_push(&meta, &staging, me)
                } else {
                    Err(GateError::Denied)
                }
            }
        }
        Some(Err(e)) => Err(e),
        None => Err(GateError::Denied),
    };
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

/// 从 hole 取一条消息：权柄判定（R）→ 长度校验 → try_pull → 锁外拷回用户。
///
/// a0 = 实际长度、a1 = 发送者 task id；发送者是内核在 Push 时盖的章，不可伪造。
fn pull(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: usize,
    buf: KVirt,
    max: usize,
) -> Outcome {
    let task = current().running_task();
    let r = match task.and_then(|t| {
        let pies = t.pies.lock();
        let pie = pies.iter().find(|p| p.token() == token)?;
        if !pie.allows(Need::Read) {
            return Some(Err(GateError::Denied));
        }
        if !pie.alive() {
            return Some(Err(GateError::Dead));
        }
        match pie {
            AnyPie::Hole(p) => Some(Ok(p.meta().clone())),
            _ => None,
        }
    }) {
        Some(Ok(meta)) => {
            if max == 0 || max > meta.mtu {
                Err(GateError::Denied)
            } else {
                let mut staging: Vec<u8> = alloc::vec![0u8; max];
                match mail::hole::try_pull(&meta, &mut staging) {
                    Ok((n, from)) => {
                        if !mail::copy_out(&ident.team.space, &staging[..n], buf.as_usize()) {
                            Err(GateError::Denied)
                        } else {
                            Ok((n, from))
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        }
        Some(Err(e)) => Err(e),
        None => Err(GateError::Denied),
    };
    // 正路径：a0 = 实际长度、a1 = 发送者 task id；错误路径 a0 = 负码。
    match r {
        Ok((n, from)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// 等某方向就绪：权柄判定（`dir` 决定 R 还是 W）→ 探测或挂起。
///
/// `millis == usize::MAX` = 永久；`0` = 只探测不挂起。a0 返 `true` = 本次调用
/// **当场就绪**（未挂起）；`false` = 未就绪（探测失败，或被唤醒/超时——两者不分）。
/// **绝不返 `-3 Busy`**：未就绪的答案就是 `false`。
fn wait_dir(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: usize,
    dir: HoleDir,
    millis: usize,
) -> Outcome {
    // 锁内解析 token → Arc<HoleMeta>：pies 与 wait_sites 同为 L3，绝不嵌套；
    // `running_task` 的临时强引用在闭包内即 drop，不跨挂起。
    let need = match dir {
        HoleDir::Pull => Need::Read,
        HoleDir::Push => Need::Write,
    };
    let resolved = match current().running_task().and_then(|t| {
        let pies = t.pies.lock();
        let pie = pies.iter().find(|p| p.token() == token)?;
        if !pie.allows(need) {
            return Some(Err(GateError::Denied));
        }
        if !pie.alive() {
            return Some(Err(GateError::Dead));
        }
        match pie {
            AnyPie::Hole(p) => Some(Ok(p.meta().clone())),
            _ => None,
        }
    }) {
        Some(r) => r,
        None => Err(GateError::Denied),
    };
    let dur = if millis == usize::MAX {
        Duration::MAX
    } else {
        Duration::from_millis(millis as u64)
    };
    match resolved {
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
        Ok(meta) => {
            // 挂起路径的默认返回 = false（未当场就绪）；可能 halt 的分支先放身份。
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            match mail::hole::wait(&meta, dir, dur) {
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
                Ok(mail::hole::Waited::Resume(true)) => frame.gpr.set_x(Gprs::A0, 1),
                Ok(mail::hole::Waited::Resume(false)) => {}
                Ok(mail::hole::Waited::Parked(Some(pa))) => {
                    return Outcome::Park(pa as *mut TrapContext);
                }
                Ok(mail::hole::Waited::Parked(None)) => {
                    return Outcome::Park(run() as *mut TrapContext);
                }
            }
        }
    }
    Outcome::Resume
}
