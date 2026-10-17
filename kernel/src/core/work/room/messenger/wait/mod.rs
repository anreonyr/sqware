pub(super) mod call;
pub(super) mod fwd;
pub(super) mod holder;
pub(super) mod key;
pub(super) mod site;

mod blocked;
mod fail;
mod unchain;

use alloc::sync::{Arc, Weak};
use core::time::Duration;

use crate::core::memory::nest::Asid;
use crate::core::work::record::trace::{self, EventKind, RoomEvent};
use crate::core::work::room::deadline;
use crate::core::work::room::conductor::current;
use crate::core::work::room::conductor::run::run;
use crate::core::work::room::scheduler::launch;
use crate::core::work::unit::life::{Life, TaskLife};
use crate::core::work::unit::task::Task;
use crate::machine::clock;

use self::blocked::{Blocked, finish_block};
use self::call::WaitCall;
use self::fail::WaitFail;
use self::fwd::Fwd;
use self::holder::{Ticket, hold, void};
use self::key::WakeKey;
use self::site::{SITE_SHARDS, Site, prune, shard_at, sites, take_beacon};
use self::unchain::Unchain;
use super::handoff::Handoff;

fn block_with_call<E: WaitFail>(
    key: WakeKey,
    life: Weak<Life>,
    dur: Duration,
    wait: Option<WaitCall>,
) -> Result<Blocked, E> {
    if take_beacon(key) {
        return Ok(Blocked::Resume);
    }
    let Some(me) = current().running_task() else {
        return Err(E::busy());
    };
    if let Some(reason) = super::take_doomed(me.ident.id) {
        drop(me);
        return Ok(Blocked::Quit(reason));
    }
    let _commit = crate::core::work::commit();
    {
        let mut sites = sites(key).lock();
        Site::ensure(&mut sites, key, &life).map_err(|()| E::oom())?;
    }
    let ticket = Ticket::next_id();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, key, &me).map_err(|()| E::oom())?;
        if deadline::tock(ticket.raw(), at).is_err() {
            void(ticket);
            return Err(E::oom());
        }
    }
    drop(me);
    let (task, next_pa) = current().swap();
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id.get(),
        key: key.fold() as usize,
    }));
    task.block(key, ticket, wait);
    let queued = {
        let mut sites = sites(key).lock();
        let queued = match sites.get_mut(&key) {
            None => false,
            Some(site) if Life::dead(&site.life) => false,
            Some(site) if site.pend => {
                site.pend = false;
                false
            }
            Some(site) => {
                site.push_back(task.clone());
                true
            }
        };
        prune(&mut sites, key);
        queued
    };

    if queued {
        drop(task);
    } else {
        void(ticket);
        rise(core::iter::once(task));
    }
    #[cfg(debug_assertions)]
    crate::core::work::unit::copies::check_block_heldout();
    Ok(Blocked::Switch(next_pa))
}

fn fan_out(fwd: &Fwd) {
    for (id, life) in fwd.entries() {
        knock(WakeKey::Tole { id }, life);
    }
}

fn rise<I: IntoIterator<Item = Arc<Task>>>(tasks: I) -> usize {
    let mut woke = 0;
    for task in tasks {
        let _commit = crate::core::work::commit();
        let t = task;
        if let Some(join) = t.take_wait() {
            let resumed = match join {
                WaitCall::Join(join) => resume_join(&t, join),
                WaitCall::Mail(wait) => resume_await(&t, wait),
            };
            if !resumed {
                continue;
            }
        }
        if !Task::rise(&t) {
            continue;
        }
        trace::note(EventKind::Room(RoomEvent::Wake {
            tid: t.ident.id.get(),
        }));
        launch(t);
        woke += 1;
    }
    woke
}

fn alarm_block<E: WaitFail>(task: Arc<Task>, wake_at: usize, dur: Duration) -> Result<usize, E> {
    let me = task.ident.id;
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me.get(),
        wake_at,
    }));
    let life = task.life();
    drop(task);
    match wait(WakeKey::Alarm { task: me }, life, dur)? {
        Handoff::Switch(pa) => Ok(pa),
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
}

pub fn park<E: WaitFail>(duration: Duration) -> Result<usize, E> {
    let Some(task) = current().running_task() else {
        return Ok(run());
    };
    let wake_at = clock::now().add(duration).as_ticks() as usize;
    alarm_block::<E>(task, wake_at, duration)
}

pub fn park_until<E: WaitFail>(at: u64) -> Result<Option<usize>, E> {
    let at_ticks = clock::duration_to_ticks(Duration::from_nanos(at));
    let now_ticks = clock::uptime_ticks();
    if at_ticks <= now_ticks {
        return Ok(None);
    }
    let Some(task) = current().running_task() else {
        return Ok(Some(run()));
    };
    let wait_ticks = at_ticks - now_ticks;
    let wake_at = (clock::now().as_ticks() + wait_ticks) as usize;
    alarm_block::<E>(task, wake_at, clock::ticks_to_duration(wait_ticks)).map(Some)
}

pub fn wait<E: WaitFail>(key: WakeKey, life: Weak<Life>, dur: Duration) -> Result<Handoff<()>, E> {
    Ok(finish_block(block_with_call(key, life, dur, None)?))
}

/// 条件等待：在同一提交边界内只跑一次纯谓词，不满足才入队。
///
/// 谓词与它捕获的上下文（`context` 里强持的 `Nest`/`View`）都在进入
/// `finish_block` 之前消费掉：那一步可能切走或进 idle，不能带着它们；
/// 谓词本身必须是纯读——它在这个提交边界内跑，不能推进失效或物化。
/// 醒后不在这里重查：调用方按原样重试那条指令，重查由下一次谓词完成。
pub(crate) fn wait_until<E, C>(
    key: WakeKey,
    life: Weak<Life>,
    context: C,
    ready: fn(&C) -> bool,
) -> Result<Handoff<()>, E>
where
    E: WaitFail,
{
    let blocked = {
        let _commit = crate::core::work::commit();
        if ready(&context) {
            None
        } else {
            Some(block_with_call::<E>(key, life, Duration::MAX, None)?)
        }
    };
    drop(context);
    Ok(match blocked {
        None => Handoff::Resume(()),
        Some(blocked) => finish_block(blocked),
    })
}

pub fn fall<E: WaitFail>(me: TaskLife, dur: Duration) -> Result<Handoff<bool>, E> {
    let TaskLife { id, life } = me;
    match wait(WakeKey::Pies { task: id }, life, dur)? {
        Handoff::Switch(pa) => Ok(Handoff::Switch(pa)),
        Handoff::Resume(()) => Ok(Handoff::Resume(true)),
    }
}

pub(crate) fn wipe(key: WakeKey) -> usize {
    let _commit = crate::core::work::commit();
    let chain = {
        let mut sites = sites(key).lock();
        sites.remove(&key)
    };
    let chain = match chain {
        Some(mut site) => {
            fan_out(&site.fwd);
            site.chain.take()
        }
        None => None,
    };
    rise(Unchain::new(chain))
}

pub(crate) fn knock(key: WakeKey, life: &Weak<Life>) -> usize {
    let _commit = crate::core::work::commit();
    let mut fwd = Fwd::empty();
    let chain = {
        let mut sites = sites(key).lock();
        let chain = match sites.get_mut(&key) {
            Some(site) => {
                fwd = site.fwd.clone();
                let chain = site.chain.take();
                if chain.is_none() {
                    site.pend = true;
                }
                chain
            }
            None => {
                if !Life::dead(life)
                    && let Ok(site) = Site::ensure(&mut sites, key, life)
                {
                    site.pend = true;
                }
                None
            }
        };
        prune(&mut sites, key);
        chain
    };
    fan_out(&fwd);
    rise(Unchain::new(chain))
}

pub(crate) fn forward(
    key: WakeKey,
    life: Weak<Life>,
    tole: usize,
    tole_life: Weak<Life>,
) -> Result<(), ()> {
    let mut sites = sites(key).lock();
    let site = Site::ensure(&mut sites, key, &life)?;
    let r = site.fwd.attach(tole, tole_life);
    prune(&mut sites, key);
    r
}

pub(crate) fn unforward(key: WakeKey, tole: usize) {
    let mut sites = sites(key).lock();
    if let Some(site) = sites.get_mut(&key) {
        site.fwd.detach(tole);
    }
    prune(&mut sites, key);
}

pub(crate) fn signal(key: WakeKey) -> usize {
    let _commit = crate::core::work::commit();
    let mut fwd = Fwd::empty();
    let chain = {
        let mut sites = sites(key).lock();
        let chain = match sites.get_mut(&key) {
            Some(site) => {
                fwd = site.fwd.clone();
                site.chain.take()
            }
            None => None,
        };
        prune(&mut sites, key);
        chain
    };
    fan_out(&fwd);
    rise(Unchain::new(chain))
}

#[cfg(debug_assertions)]
pub(crate) fn site_count() -> usize {
    (0..SITE_SHARDS)
        .map(|shard| shard_at(shard).lock().len())
        .sum()
}

/// 这个等待位此刻排着几名真实等待者。
///
/// 只读一边的队列长：不推进失效、不物化、不分配、不改排队次序，
/// 故可以在提交边界之外当“它真登记了没有”的读口用。
#[cfg(all(debug_assertions, feature = "fresh-scene"))]
pub(crate) fn queued(key: WakeKey) -> usize {
    let sites = sites(key).lock();
    sites.get(&key).map_or(0, |site| site.chain.len())
}

pub(crate) fn wipe_space(space: Asid) -> usize {
    let _commit = crate::core::work::commit();
    let mut woken = 0usize;
    for shard in 0..SITE_SHARDS {
        loop {
            let taken = {
                let mut sites = shard_at(shard).lock();
                let key = sites
                    .keys()
                    .find(|k| {
                        matches!(
                            k,
                            WakeKey::Space { space: s, .. } | WakeKey::View { space: s, .. }
                                if *s == space
                        )
                    })
                    .copied();
                key.and_then(|key| sites.remove(&key))
            };
            let Some(mut site) = taken else { break };
            woken += rise(Unchain::new(site.chain.take()));
        }
    }
    woken
}

pub fn wake(key: WakeKey, life: &Weak<Life>) -> bool {
    let _commit = crate::core::work::commit();
    debug_assert!(
        !matches!(key, WakeKey::Tole { .. }),
        "组键要走 knock（提示型/整链放行），不能走 wake（交付型/一人）"
    );
    let (popped, fwd) = {
        let mut sites = sites(key).lock();
        if Life::dead(life) {
            let chain = sites.remove(&key).and_then(|mut site| site.chain.take());
            drop(sites);
            rise(Unchain::new(chain));
            return false;
        } else {
            let mut beacon_only = false;
            let popped = match sites.get_mut(&key) {
                Some(site) => match site.chain.pop() {
                    Some(task) => Some(task),
                    None => {
                        site.pend = true;
                        None
                    }
                },
                None => {
                    beacon_only = true;
                    None
                }
            };
            if beacon_only && let Ok(site) = Site::ensure(&mut sites, key, life) {
                site.pend = true;
            }
            let fwd = sites.get(&key).map_or_else(Fwd::empty, |s| s.fwd.clone());
            prune(&mut sites, key);
            (popped, fwd)
        }
    };
    fan_out(&fwd);
    let Some(task) = popped else { return false };
    void(task.wait_ticket());
    rise(core::iter::once(task));
    true
}

pub fn redeem() -> bool {
    let _commit = crate::core::work::commit();
    const MAX_DUE: usize = 64;
    let mut due = [0u64; MAX_DUE];
    let n = deadline::drain(clock::now(), &mut due);
    let mut tasks: [Option<Arc<Task>>; MAX_DUE] = [const { None }; MAX_DUE];
    let mut woken = 0usize;
    for (slot, &handle) in tasks.iter_mut().zip(&due[..n]) {
        let Some((key, holder)) = void(Ticket(handle)) else {
            continue;
        };
        drop(holder);
        let popped = {
            let mut sites = sites(key).lock();
            let pick = &mut |t: &mut Arc<Task>| t.wait_ticket() == Ticket(handle);
            let w = sites
                .get_mut(&key)
                .and_then(|site| site.chain.remove_if(pick));
            prune(&mut sites, key);
            w
        };
        let Some(w) = popped else { continue };
        *slot = Some(w);
        woken += 1;
    }
    let _ = woken;
    rise(tasks.iter_mut().filter_map(Option::take)) > 0
}

pub(crate) fn unit_join(
    join: crate::core::work::unit::join::JoinWait,
) -> Result<Handoff<env::JoinReply>, env::UnitFail> {
    loop {
        let _commit = crate::core::work::commit();
        let reply = join.poll()?;
        if reply != env::JoinReply::Pending || join.remaining() == Duration::ZERO {
            return Ok(Handoff::Resume(reply));
        }
        let blocked = block_with_call::<env::UnitFail>(
            join.target.key(),
            join.target.life(),
            join.remaining(),
            Some(WaitCall::Join(join.clone())),
        )?;
        drop(_commit);
        match finish_block(blocked) {
            Handoff::Resume(()) => continue,
            Handoff::Switch(pa) => return Ok(Handoff::Switch(pa)),
        }
    }
}
fn resume_join(task: &Arc<Task>, join: crate::core::work::unit::join::JoinWait) -> bool {
    use crate::machine::switcher::context::TrapContext;
    use env::JoinReply;
    let _commit = crate::core::work::commit();
    let result = join.poll();
    let result = if matches!(result, Ok(JoinReply::Pending)) && join.remaining() != Duration::ZERO {
        match requeue(
            task,
            join.target.key(),
            join.target.life(),
            join.remaining(),
            WaitCall::Join(join.clone()),
            env::UnitFail::OoM,
        ) {
            Ok(()) => return false,
            Err(error) => Err(error),
        }
    } else {
        result
    };
    let frame = unsafe { &mut *(task.ident.frame_pa.as_usize() as *mut TrapContext) };
    crate::core::work::unit::join::JoinWait::write(frame, result);
    true
}
fn requeue<E: Copy>(
    task: &Arc<Task>,
    key: WakeKey,
    life: Weak<Life>,
    dur: Duration,
    call: WaitCall,
    oom: E,
) -> Result<(), E> {
    {
        let mut table = sites(key).lock();
        Site::ensure(&mut table, key, &life).map_err(|()| oom)?;
    }
    let ticket = Ticket::next_id();
    if dur != Duration::MAX {
        hold(ticket, key, task).map_err(|_| oom)?;
        if deadline::tock(ticket.raw(), clock::now().add(dur).as_ticks()).is_err() {
            void(ticket);
            return Err(oom);
        }
    }
    task.block(key, ticket, Some(call));
    let mut table = sites(key).lock();
    let site = table.get_mut(&key).expect("requeued site");
    site.pend = false;
    site.push_back(task.clone());
    Ok(())
}

pub(crate) fn mail_await(
    wait: crate::core::work::mail::tole::AwaitWait,
) -> Result<Handoff<env::AwaitReply>, env::MailFail> {
    loop {
        let commit = crate::core::work::commit();
        let reply = match wait.poll() {
            Ok(reply) => reply,
            Err(e) => {
                wait.unwatch();
                return Err(e);
            }
        };
        if reply != env::AwaitReply::Pending || wait.remaining() == Duration::ZERO {
            wait.unwatch();
            return Ok(Handoff::Resume(reply));
        }
        wait.watch()?;
        let blocked = match block_with_call::<env::MailFail>(
            wait.key(),
            wait.life(),
            wait.remaining(),
            Some(WaitCall::Mail(wait.clone())),
        ) {
            Ok(blocked) => blocked,
            Err(e) => {
                wait.unwatch();
                return Err(e);
            }
        };
        drop(commit);
        match finish_block(blocked) {
            Handoff::Resume(()) => continue,
            Handoff::Switch(pa) => return Ok(Handoff::Switch(pa)),
        }
    }
}
fn resume_await(task: &Arc<Task>, wait: crate::core::work::mail::tole::AwaitWait) -> bool {
    let _commit = crate::core::work::commit();
    let result = wait.poll();
    let result =
        if matches!(result, Ok(env::AwaitReply::Pending)) && wait.remaining() != Duration::ZERO {
            match requeue(
                task,
                wait.key(),
                wait.life(),
                wait.remaining(),
                WaitCall::Mail(wait.clone()),
                env::MailFail::OoM,
            ) {
                Ok(()) => return false,
                Err(e) => Err(e),
            }
        } else {
            result
        };
    let frame = unsafe {
        &mut *(task.ident.frame_pa.as_usize()
            as *mut crate::machine::switcher::context::TrapContext)
    };
    wait.unwatch();
    crate::core::work::mail::tole::AwaitWait::write(frame, result);
    true
}
