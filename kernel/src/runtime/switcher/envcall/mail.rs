use alloc::sync::Arc;

use env::{HoleDir, MailCall, MailFail, PieToken, TaskId, Wait};

use riscv::register::sie;

use crate::memory::manager::entry::PteFlags;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, AnyPie, Need};
use crate::work::unit::space::Space;

use super::pie::usable;
use crate::work::unit::task::TaskIdent;

pub(crate) enum Outcome {
    Resume,
    Park(*mut TrapContext),
}

pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: MailCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    Some(match call {
        MailCall::Push { token, msg, len } => push(frame, ident, token, msg.get(), len),
        MailCall::Pull { token, buf, max } => pull(frame, ident, token, buf.get(), max),
        MailCall::Peek { token } => peek(frame, token),
        MailCall::Wait { token, dir, millis } => wait_dir(frame, ident, token, dir, millis),
        MailCall::Hush { token } => hush(frame, token),
        MailCall::Ring { token } => ring(frame, token),
    })
}

fn push(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    msg: usize,
    len: usize,
) -> Outcome {
    let me = current()
        .running_task()
        .map(|t| t.ident.id)
        .unwrap_or(TaskId::new(0));
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, Need::Store));
    let r = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                // **这一句是"字节归内核"的分界**：段表在这里先走一遍（不合答 `Denied`——
                // 那是**你自己的错**），当场抄一份进内核那一格；此后这一段与发送方无关，
                // 取的一方也不必再翻它的页表。
                AnyPie::Hole(p) => {
                    if len == 0 {
                        Err(MailFail::Denied)
                    } else if !mail::whole(&ident.team.space, msg, len, PteFlags::R) {
                        Err(MailFail::Denied)
                    } else {
                        (|| {
                            let reservation = mail::hole::reserve(p.meta())?;
                            let mut cell = alloc::vec::Vec::new();
                            cell.try_reserve_exact(len).map_err(|_| MailFail::OoM)?;
                            cell.resize(len, 0);
                            if !mail::copy_in(&ident.team.space, &mut cell, msg) {
                                return Err(MailFail::Denied);
                            }
                            let bytes = Arc::try_new(cell).map_err(|_| MailFail::OoM)?;
                            reservation.commit(bytes, me)
                        })()
                    }
                }
                _ => Err(MailFail::Denied),
            },
        },
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

fn pull(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    buf: usize,
    max: usize,
) -> Outcome {
    if !Space::user_range(buf, max) {
        frame.gpr.set_x(Gprs::A0, MailFail::Denied.code() as usize);
        return Outcome::Resume;
    }
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, Need::Fetch));
    let r = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => hand_over(p.meta(), &ident.team.space, buf, max),
                _ => Err(MailFail::Denied),
            },
        },
    };
    match r {
        Ok((n, from)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// **取走一只手**：认下它（`take`：置"正被取用"）→ 复制**一次** → 收尾（`taken`）。
///
/// 两条不过的路**都不消费那只手**（`back` 放回原处）：装不下、收方缓冲不可写 ⇒ `Denied`。
/// **"发送方那段没了"那一档没有了**：手里那段字节是 `Push` 时抄进内核的，与发送方无关。
///
/// **复制在孔锁之外做**：`Space` 的锁是 `Level::Space`（2）、孔那一格是 `Level::L3`（4），
/// 持孔锁再取空间锁是倒序（debug 档 lockdep 当场报），而复制每页都要过一遍 `translate`。
fn hand_over(
    meta: &Arc<mail::HoleMeta>,
    space: &Arc<Space>,
    buf: usize,
    max: usize,
) -> Result<(usize, TaskId), MailFail> {
    mail::hole::take(meta)?;
    let Some((from, bytes)) = mail::hole::source(meta) else {
        mail::hole::back(meta);
        return Err(MailFail::Busy);
    };
    let len = bytes.len();
    if len > max || !mail::whole(space, buf, len, PteFlags::W) {
        // **"读不成、手放回"这一条要看得见**（诊断）：孔会因此**一直报就绪**——等在这一组上的
        // 读的人每一轮都啃同一格（读不成 ⇒ 手还在 ⇒ 下一轮又报就绪），而**别的客人的手就被饿在
        // 后面**；递手的那一方还在等它下线（`wait(HoleDir::Push, …)`）⇒ 两边一起卡住。
        mail::hole::note_back(len, max);
        mail::hole::back(meta);
        return Err(MailFail::Denied);
    }
    if !mail::copy_out(space, &bytes, buf) {
        // 收方那段写不进去（同一格的另一个成因）：手放回原处，下一次换够大的缓冲再来。
        mail::hole::note_back(len, max);
        mail::hole::back(meta);
        return Err(MailFail::Denied);
    }
    mail::hole::taken(meta);
    Ok((len, from))
}

/// 只看那只手：`(长度, 发送者, 队里排着几只)`。不动状态、不唤醒、不复制。
///
/// 第三格（A2）此前空着：单槽时代"排着几只"恒为一件事，没有可报的。队列化之后它是写者
/// 唯一的凭据——"我还排着几手"（`hand::Sender` 那一侧靠它把缓冲收回来）。
fn peek(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Fetch, |pie| match pie {
        AnyPie::Hole(p) => mail::hole::peek(p.meta()),
        _ => Err(MailFail::Denied),
    });
    match r {
        Ok((n, from, depth)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
            frame.gpr.set_x(Gprs::A2, depth);
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// 等某一方向就绪（`Pull` 有可取之事／`Push` 孔空着），`millis` 是上限族。
fn wait_dir(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    dir: HoleDir,
    millis: Wait,
) -> Outcome {
    enum Ready {
        Hole(Arc<mail::hole::HoleMeta>),
        Bell(Arc<mail::nole::NoleMeta>),
        /// **页上那一位**（架把铃并进页；只有 `Pull` 一条方向，与门铃同）。
        Page(Arc<mail::pole::PoleMeta>),
    }
    let need = match dir {
        HoleDir::Pull => Need::Fetch,
        HoleDir::Push => Need::Store,
    };
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, need));
    let resolved = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => Ok(Ready::Hole(p.meta().clone())),
                AnyPie::Nole(p) if dir == HoleDir::Pull => Ok(Ready::Bell(p.meta().clone())),
                // 页只有"有事"一条方向（与门铃同一条纪律：别的 `dir` 答 `Denied`）。
                AnyPie::Pole(p) if dir == HoleDir::Pull => Ok(Ready::Page(p.meta().clone())),
                _ => Err(MailFail::Denied),
            },
        },
    };
    let dur = millis.into_duration();
    match resolved {
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
        Ok(ready) => {
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            let parked = match &ready {
                Ready::Hole(meta) => mail::hole::wait(meta, dir, dur),
                Ready::Bell(meta) => mail::nole::wait(meta, dur),
                Ready::Page(meta) => mail::pole::wait(meta, dur),
            };
            match parked {
                Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
                Ok(Handoff::Switch(pa)) => return Outcome::Park(pa as *mut TrapContext),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
    }
    Outcome::Resume
}

fn hush(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Fetch, |pie| match pie {
        AnyPie::Nole(p) => {
            let r = mail::nole::hush(p.meta());
            if r.is_ok() {
                // SAFETY: 仅置本 hart SEIE 位
                unsafe {
                    sie::set_sext();
                }
            }
            r
        }
        // 孔上那一位不是中断响的：**不碰闸门**。
        AnyPie::Hole(p) => mail::hole::hush(p.meta()),
        // **页上那一位同样不是中断响的**：照孔那一支写，不 `set_sext`。
        // （这正是"铃并进页"的一个好处：驱动那颗 hart 的闸门仍只由 `line.exhaust()` 那一手重开。）
        AnyPie::Pole(p) => mail::pole::hush(p.meta()),
        _ => Err(MailFail::Denied),
    });
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

fn ring(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_pie(token, Need::Store, |pie| match pie {
        AnyPie::Nole(p) => mail::nole::ring(p.meta()),
        AnyPie::Hole(p) => mail::hole::ring(p.meta()),
        // 页上那一位：架的写端每落一格响一下（已响答 `Busy`，写者当"正好"）。
        AnyPie::Pole(p) => mail::pole::ring(p.meta()),
        _ => Err(MailFail::Denied),
    });
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

/// 认一枚门闩、验权、把 `op` 作用上去——门铃与孔上那一位共用这两手。
fn with_pie<T>(
    token: PieToken,
    need: Need,
    op: impl FnOnce(&AnyPie) -> Result<T, MailFail>,
) -> Result<T, MailFail> {
    let found = current()
        .running_task()
        .ok_or(MailFail::Denied)
        .and_then(|t| gate::accede::<MailFail>(&t, token, need));
    match found {
        Err(e) => Err(e),
        Ok(pie) => match usable::<MailFail>(&pie) {
            Err(e) => Err(e),
            Ok(()) => op(&pie),
        },
    }
}
