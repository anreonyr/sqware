use alloc::sync::{Arc, Weak};
use core::sync::atomic::Ordering;

use env::{MailCondition, ToleCall};

use env::{PieToken, Source, TaskId, ToleFail, Wait};

use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail::tole::{MATE_SKIP, Mate, Sub};
use crate::work::mail::{ToleMeta, tole};
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::{self, Need, Permission, PieSnapshot};
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
        ToleCall::Attach {
            tole,
            pie,
            condition,
        } => attach(frame, tole, pie, condition),
        ToleCall::Detach {
            tole,
            pie,
            condition,
        } => detach(frame, tole, pie, condition),
        ToleCall::Await { tole, millis } => return Some(await_(frame, tole, millis)),
        ToleCall::Subscribe {
            tole,
            source,
            target,
        } => subscribe(frame, tole, source, target),
        ToleCall::Unsubscribe {
            tole,
            source,
            target,
        } => unsubscribe(frame, tole, source, target),
    })
}

fn attach(
    frame: &mut TrapContext,
    group: PieToken,
    member: PieToken,
    dir: MailCondition,
) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        let latch = gate::accede::<ToleFail>(
            &task,
            member,
            if matches!(dir, MailCondition::Pull | MailCondition::Signal(_)) {
                Need::Fetch
            } else {
                Need::Store
            },
        )?;
        usable::<ToleFail>(&latch)?;
        let (mate, life) = mate(&latch, dir)?;
        tole::attach(&meta, mate, life)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

fn detach(
    frame: &mut TrapContext,
    group: PieToken,
    member: PieToken,
    dir: MailCondition,
) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        let latch = gate::accede::<ToleFail>(
            &task,
            member,
            if matches!(dir, MailCondition::Pull | MailCondition::Signal(_)) {
                Need::Fetch
            } else {
                Need::Store
            },
        )?;
        let (mate, _life) = mate(&latch, dir)?;
        tole::detach(&meta, mate)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

/// 把一个状态来源登记进组。
///
/// 次序即判据：组的 `STORE` → 组必须是**本地持有的独占组** → 来源与目标这一对必须合法
/// → 取来源那一侧的弱寿命 → 组层登记（幂等、失败回滚、成功留一次待复核提示）。
fn subscribe(frame: &mut TrapContext, group: PieToken, source: Source, target: TaskId) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        // 首版只收本地持有的独占组：含状态订阅的组一旦易主，"观察我自己"那一格就与新持有者
        // 错配（转授那一侧另有拒授，见 `gate::accord`）。
        if !latch.permission().contains(Permission::ONLY) {
            return Err(ToleFail::Denied);
        }
        let (sub, life) = match source {
            Source::CapabilitiesChanged => {
                if target != task.ident.id {
                    return Err(ToleFail::Denied);
                }
                (Sub::Capabilities(target), task.life())
            }
            Source::TaskCompleted => match muster(target).and_then(|w| w.upgrade()) {
                Some(observed) => {
                    // 观察范围不得比 `Join` 宽：逐条复用它的那一条授权判据。
                    let same = Arc::ptr_eq(&observed.ident.team, &task.ident.team);
                    if !(same || task.heir(observed.ident.team.id).is_some()) {
                        return Err(ToleFail::Denied);
                    }
                    (Sub::TaskCompleted(target), observed.life())
                }
                // 目标已经升不出来：授权无从核，但这条边也建不起来——`forward` 收尾那一趟
                // `prune` 会用已死的弱寿命把站点收掉。于是这一格退化成"自己组上响一声"：
                // 初始提示保证调用方复核一次，而没有任何持久的观察能力被授出。
                None => (Sub::TaskCompleted(target), Weak::new()),
            },
        };
        tole::subscribe(&meta, sub, life)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

/// 按**已安装的订阅描述**取消：只认那条描述，不核组合、不核目标死活。
fn unsubscribe(
    frame: &mut TrapContext,
    group: PieToken,
    source: Source,
    target: TaskId,
) -> Outcome {
    let r = (|| -> Result<(), ToleFail> {
        let task = current().running_task().ok_or(ToleFail::Denied)?;
        let latch = gate::accede::<ToleFail>(&task, group, Need::Store)?;
        let meta = rack(&latch)?;
        let sub = match source {
            Source::TaskCompleted => Sub::TaskCompleted(target),
            Source::CapabilitiesChanged => Sub::Capabilities(target),
        };
        tole::unsubscribe(&meta, sub)
    })();
    answer_void(frame, r);
    Outcome::Resume
}

fn await_(frame: &mut TrapContext, group: PieToken, millis: Wait) -> Outcome {
    let dur = millis.into_duration();
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
    answer_pair(frame, PieToken::NONE, MailCondition::Pull);
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
fn ready(meta: &ToleMeta) -> (Option<(PieToken, MailCondition)>, usize) {
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
    let pies = task.gate.pies.lock();
    for k in 0..count {
        let at = (start + k) % count;
        match cells[at].mate() {
            Mate::Hole(id, dir) => {
                let Some(pie) = pies.iter().find(|p| {
                    p.alive()
                        && p.heir().is_none()
                        && p.allows(if dir == MailCondition::Pull {
                            Need::Fetch
                        } else {
                            Need::Store
                        })
                        && p.snapshot().hole().is_some_and(|meta| meta.id() == id)
                }) else {
                    // **成员还在组里、可本域表里已经没有那一枚了** ⇒ 这一格**永远报不出就绪**。
                    // 这一格**不被吞掉**（不 `continue`）——"组里有人、读的人却一直睡"这件事
                    // 由此落到读数上。数下来，第一次当场报一行（见 [`await_`]）。
                    skipped += 1;
                    continue;
                };
                let Some(h) = pie.snapshot().hole() else {
                    continue;
                };
                if h.ready(dir) {
                    meta.seek_cursor((at + 1) % count);
                    return (Some((pie.token(), dir)), skipped);
                }
            }
            Mate::Nole(id) => {
                let Some(pie) = pies.iter().find(|p| {
                    p.alive()
                        && p.heir().is_none()
                        && p.allows(Need::Fetch)
                        && p.snapshot().nole().is_some_and(|meta| meta.id() == id)
                }) else {
                    skipped += 1;
                    continue;
                };
                let Some(n) = pie.snapshot().nole() else {
                    continue;
                };
                if n.ready() {
                    meta.seek_cursor((at + 1) % count);
                    return (Some((pie.token(), MailCondition::Pull)), skipped);
                }
            }
            // **页上那一位**（架把铃并进页）：与 `Mate::Nole` 同一形，方向恒为 `Pull`。
            Mate::Pole(id, bit) => {
                let Some(pie) = pies.iter().find(|p| {
                    p.alive()
                        && p.heir().is_none()
                        && p.allows(Need::Fetch)
                        && p.snapshot().pole().is_some_and(|meta| meta.id() == id)
                }) else {
                    skipped += 1;
                    continue;
                };
                let Some(p) = pie.snapshot().pole() else {
                    continue;
                };
                if p.ready(bit) {
                    meta.seek_cursor((at + 1) % count);
                    return (Some((pie.token(), MailCondition::Signal(bit))), skipped);
                }
            }
        }
    }
    (None, skipped)
}

fn rack(pie: &PieSnapshot) -> Result<Arc<ToleMeta>, ToleFail> {
    pie.tole().ok_or(ToleFail::Denied)
}

fn mate(pie: &PieSnapshot, dir: MailCondition) -> Result<(Mate, Weak<Life>), ToleFail> {
    if let Some(h) = pie.hole() {
        if matches!(dir, MailCondition::Signal(_)) { return Err(ToleFail::Denied); }
        return Ok((Mate::Hole(h.id(), dir), h.life()));
    }
    if dir == MailCondition::Pull {
        if let Some(n) = pie.nole() {
            return Ok((Mate::Nole(n.id()), n.life()));
        }
    }
    if let MailCondition::Signal(bit) = dir {
        if let Some(p) = pie.pole() { return Ok((Mate::Pole(p.id(), bit), p.life())); }
    }
    Err(ToleFail::Denied)
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

fn answer_pair(frame: &mut TrapContext, token: PieToken, dir: MailCondition) {
    frame.gpr.set_x(Gprs::A0, token.get());
    frame.gpr.set_x(Gprs::A1, dir.wire());
}
