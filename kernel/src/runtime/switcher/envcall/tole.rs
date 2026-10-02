use alloc::sync::{Arc, Weak};
use core::sync::atomic::Ordering;

use env::{HoleDir, Mark, ToleCall};

use env::{PieToken, ToleFail, Wait};

use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail::tole::{MATE_SKIP, Mate};
use crate::work::mail::{ToleMeta, tole};
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, AnyPie, Need, Permission, Pie};
use crate::work::unit::life::Life;
use crate::work::unit::task::TaskIdent;

use super::mail::Outcome;
use super::pie::usable;

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
        ToleCall::Await { tole, millis } => return Some(await_(frame, tole, millis)),
    })
}

fn unseal(frame: &mut TrapContext, shared: bool) -> Outcome {
    let r = (|| -> Result<usize, ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let meta = tole::meta(task.ident.id);
        // **（临时读数）组号 ↔ 主人**：`wake … first=`／`knock tole#…` 里报的都是**组号**，
        // 而"那是不是等这一枚的那一位"只能与这一行对。前 40 次打全（每组只铸一次）。
        {
            static N: ::core::sync::atomic::AtomicUsize =
                ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 40 {
                crate::putln!(
                    "unseal tole#{} owner={}",
                    meta.id().0,
                    task.ident.id.get()
                );
            }
        }
        let mut latch = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            latch |= Permission::ONLY;
        }
        let pie: Pie<gate::Tole> = gate::new_pie(meta, Mark::NONE, latch, None);
        let token = pie.token;
        let mut pies = task.pies.lock();
        pies.try_reserve(1).map_err(|_| ToleFail::OoM)?;
        pies.push(AnyPie::Tole(pie));
        Ok(token.get())
    })();
    answer(frame, r);
    Outcome::Resume
}

fn attach(frame: &mut TrapContext, group: PieToken, member: PieToken, dir: HoleDir) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        let latch = gate::accede::<ToleFail>(&task, member, Need::Fetch)?;
        usable::<ToleFail>(&latch)?;
        let (mate, life) = mate(&latch, dir)?;
        // **（临时读数）哪一枚孔挂进了组**：孔号是**内核全局编号**（`alloc_id`），故它能与
        // `ring hole#…` 那一边**跨任务对号**——"投递摇的那一枚"与"挂进组的那一枚"是不是同一枚，
        // 只能这样对。前 24 次打全。
        {
            static N: ::core::sync::atomic::AtomicUsize =
                ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 400 {
                if let Mate::Hole(h, d) = mate {
                    crate::putln!(
                        "attach hole#{} dir={:?} task={}",
                        h.0,
                        d,
                        task.ident.id.get()
                    );
                }
            }
        }
        tole::attach(&meta, mate, life)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

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

fn await_(frame: &mut TrapContext, group: PieToken, millis: Wait) -> Outcome {
    let dur = millis.into_duration();
    // **（临时读数）"被踢之后多久才真跑起来"**：这一位上一趟等在组上、被唤醒了（`rise` 记过
    // 时刻），而"醒"与"跑"之间那一段完全看不见（运行队列 → 调度挑中 → 切上下文）。≥100 ms 才报，
    // 前 20 次——它与 `mail: hand stuck` 的 `age` 对得上时，"敲来了、可它几秒后才跑"就是原文。
    if let Some(task) = current().running_task() {
        let id = task.ident.id.get();
        drop(task);
        if let Some(ms) = crate::work::room::messenger::kick_lag(id)
            && ms >= 100
        {
            static N: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                crate::putln!("tole: lag task={} ms={}", id, ms);
            }
        }
    }
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
    let (hit, skipped) = ready(&meta);
    // **组里有人、却没人认得出来**：这一格**不被吞掉**（不 `continue`），
    // 后果由"读的人一直睡"领（见 [`ready`]）。第一次当场报一行。
    if hit.is_none() && skipped > 0 && MATE_SKIP.fetch_add(skipped, Ordering::Relaxed) == 0 {
        crate::putln!("tole: mate skipped n={}", skipped);
    }
    if let Some((token, dir)) = hit {
        answer_pair(frame, token, dir);
        return Outcome::Resume;
    }
    answer_pair(frame, PieToken::NONE, HoleDir::Pull);
    match tole::wait(&meta, dur) {
        Ok(Handoff::Resume(())) => {
            let (hit, skipped) = ready(&meta);
            if hit.is_none() && skipped > 0 && MATE_SKIP.fetch_add(skipped, Ordering::Relaxed) == 0
            {
                crate::putln!("tole: mate skipped n={}", skipped);
            }
            if let Some((token, dir)) = hit {
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

/// **挑"哪一格有事"**：从**轮转游标**起扫一圈，取第一枚就绪的；命中之后把游标推到命中项的
/// 下一格。返 `(命中的那一枚, 跳过的格数)`。
fn ready(meta: &ToleMeta) -> (Option<(PieToken, HoleDir)>, usize) {
    let mut skipped = 0usize;
    let Some(task) = current().running_task() else {
        return (None, 0);
    };
    let cells = meta.cells();
    if cells.is_empty() {
        return (None, 0);
    }
    // **格数叫 `count`**（`Mate::Nole(n)` 那一支里有个同名绑定——叫 `n` 会被它遮住）。
    let count = cells.len();
    let start = meta.cursor() % count;
    let pies = task.pies.lock();
    for k in 0..count {
        let at = (start + k) % count;
        match cells[at].mate() {
            Mate::Hole(id, dir) => {
                let Some(pie) = pies
                    .iter()
                    .find(|p| matches!(p, AnyPie::Hole(h) if h.meta().id() == id))
                else {
                    // **成员还在组里、可本域表里已经没有那一枚了** ⇒ 这一格**永远报不出就绪**。
                    // 这一格**不被吞掉**（不 `continue`）——"组里有人、读的人却一直睡"这件事
                    // 由此落到读数上。数下来，第一次当场报一行（见 [`await_`]）。
                    //
                    // **（临时读数）是哪一枚、等的人是谁**：孔号是内核全局编号（`alloc_id`），
                    // 故它能与 `attach hole#…`／`ring hole#…` 两处**跨任务对号**。前 6 次打全。
                    {
                        static N: ::core::sync::atomic::AtomicUsize =
                            ::core::sync::atomic::AtomicUsize::new(0);
                        if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 6 {
                            crate::putln!(
                                "tole: skip hole#{} waiter={} dir={:?}",
                                id.0,
                                task.ident.id.get(),
                                dir
                            );
                        }
                    }
                    skipped += 1;
                    continue;
                };
                let AnyPie::Hole(h) = pie else { continue };
                if h.meta().ready(dir) {
                    meta.seek_cursor((at + 1) % count);
                    return (Some((pie.token(), dir)), skipped);
                }
                // **（临时读数）"有手却报不出就绪"**：`Pull` 一格里有手、而 `ready` 答否 ⇒ 只有
                // 一种成因——队头**卡在"正被取用"**上（`take` 走过、`taken`/`back` 没回来 ⇒ 这一格
                // 从此**永久**报不出就绪，整组跟着睡死）。第一行就是那件事的原文，故前 8 次打全。
                {
                    let (hands, taking, _rung) = h.meta().shape();
                    if dir == HoleDir::Pull && hands > 0 {
                        static N: ::core::sync::atomic::AtomicUsize =
                            ::core::sync::atomic::AtomicUsize::new(0);
                        if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 8 {
                            crate::putln!(
                                "tole: none hole#{} dir={:?} hands={} taking={} waiter={}",
                                id.0,
                                dir,
                                hands,
                                taking,
                                task.ident.id.get()
                            );
                        }
                    }
                }
            }
            Mate::Nole(id) => {
                let Some(pie) = pies
                    .iter()
                    .find(|p| matches!(p, AnyPie::Nole(n) if n.meta().id() == id))
                else {
                    skipped += 1;
                    continue;
                };
                let AnyPie::Nole(n) = pie else { continue };
                if n.meta().ready() {
                    meta.seek_cursor((at + 1) % count);
                    return (Some((pie.token(), HoleDir::Pull)), skipped);
                }
            }
        }
    }
    (None, skipped)
}

fn rack(pie: &AnyPie) -> Result<Arc<ToleMeta>, ToleFail> {
    match pie {
        AnyPie::Tole(t) => Ok(t.meta().clone()),
        _ => Err(ToleFail::Denied),
    }
}

fn mate(pie: &AnyPie, dir: HoleDir) -> Result<(Mate, Weak<Life>), ToleFail> {
    match pie {
        AnyPie::Hole(h) => Ok((Mate::Hole(h.meta().id(), dir), h.meta().life())),
        AnyPie::Nole(n) if dir == HoleDir::Pull => Ok((Mate::Nole(n.meta().id()), n.meta().life())),
        _ => Err(ToleFail::Denied),
    }
}

fn answer(frame: &mut TrapContext, r: Result<usize, ToleFail>) {
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(v) => v,
            Err(e) => e.code() as usize,
        },
    );
}

fn answer_void(frame: &mut TrapContext, r: Result<(), ToleFail>) {
    answer(frame, r.map(|()| 0));
}

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
