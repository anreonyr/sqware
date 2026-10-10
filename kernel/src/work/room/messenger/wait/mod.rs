pub(super) mod holder;
pub(super) mod site;

use alloc::sync::{Arc, Weak};
use core::time::Duration;

use crate::memory::manager::asid::Asid;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::{current, kick};
use crate::work::room::scheduler::trap::run;
use crate::work::unit::life::{Life, TaskLife};
use crate::work::unit::task::{Task, TaskState};

use self::holder::{Ticket, hold, void};
use self::site::{Fwd, SITE_SHARDS, Site, WakeKey, prune, shard_at, sites, take_beacon};
use super::handoff::Handoff;

pub(crate) trait WaitFail: env::FailCode {
    fn busy() -> Self;
    fn oom() -> Self;
}

impl WaitFail for env::RoomFail {
    fn busy() -> Self {
        env::RoomFail::Busy
    }
    fn oom() -> Self {
        env::RoomFail::OoM
    }
}

impl WaitFail for env::UnitFail {
    fn busy() -> Self {
        env::UnitFail::Busy
    }
    fn oom() -> Self {
        env::UnitFail::OoM
    }
}

impl WaitFail for env::MailFail {
    fn busy() -> Self {
        env::MailFail::Busy
    }
    fn oom() -> Self {
        env::MailFail::OoM
    }
}


fn block<E: WaitFail>(key: WakeKey, life: Weak<Life>, dur: Duration) -> Result<Handoff<()>, E> {
    Ok(finish_block(block_with_call(key, life, dur, None)?))
}

enum Blocked {
    Resume,
    Switch(Option<usize>),
    Quit(usize),
}
fn finish_block(blocked: Blocked) -> Handoff<()> {
    match blocked {
        Blocked::Resume => Handoff::Resume(()),
        Blocked::Switch(next) => Handoff::Switch(next.unwrap_or_else(run)),
        Blocked::Quit(reason) => {
            super::set_exit_reason(reason);
            Handoff::Switch(super::quit())
        }
    }
}

fn block_with_call<E: WaitFail>(key: WakeKey, life: Weak<Life>, dur: Duration, wait: Option<WaitCall>) -> Result<Blocked, E> {
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
    let _commit = crate::work::unit::commit();
    {
        let mut sites = sites(key).lock();
        if !sites.contains_key(&key) {
            sites.try_reserve(1).map_err(|_| E::oom())?;
            let mut site = Site::new(&Weak::new());
            site.life = life;
            sites.insert(key, site);
        }
    }
    let ticket = Ticket::alloc();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, key, &me).map_err(|()| E::oom())?;
        if timer::tock(ticket.raw(), at).is_err() {
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
    let stopped = task.stopped();
    let state = if stopped { TaskState::Debarked { state: crate::work::unit::task::TaskStopped::Blocked {
        key, ticket, next: None, wait,
    } } } else { TaskState::Blocked { key, ticket, next: None, wait } };
    *task.state.lock() = state;
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
    crate::work::unit::weak::check_block_heldout();
    Ok(Blocked::Switch(next_pa))
}

fn rise<I: IntoIterator<Item = Arc<Task>>>(tasks: I) -> usize {
    let mut woke = 0;
    for task in tasks {
        let _commit = crate::work::unit::commit();
        let t = task;
        if let Some(join) = t.take_wait() {
            let resumed = match join { WaitCall::Join(join) => resume_join(&t, join), WaitCall::Mail(wait) => resume_await(&t, wait) };
            if !resumed { continue; }
        }
        if !Task::rise(&t) { continue; }
        trace::note(EventKind::Room(RoomEvent::Wake {
            tid: t.ident.id.get(),
        }));
        kick(conductor::pick(), t);
        woke += 1;
    }
    woke
}

struct Unchain {
    cur: Option<Arc<Task>>,
}

impl Iterator for Unchain {
    type Item = Arc<Task>;

    fn next(&mut self) -> Option<Arc<Task>> {
        let mut task = self.cur.take()?;
        void(Task::blocked_ticket(&task));
        self.cur = Task::take_blocked_next(&mut task);
        Some(task)
    }
}

pub fn park<E: WaitFail>(duration: Duration) -> Result<usize, E> {
    let Some(task) = current().running_task() else {
        return Ok(run());
    };
    let me = task.ident.id;
    let wake_at = clock::now().add(duration).as_ticks();
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me.get(),
        wake_at: wake_at as usize,
    }));
    let life = task.life();
    drop(task);
    match block(WakeKey::Alarm { task: me }, life, duration)? {
        Handoff::Switch(pa) => Ok(pa),
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
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
    let me = task.ident.id;
    let wait_ticks = at_ticks - now_ticks;
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me.get(),
        wake_at: (clock::now().as_ticks() + wait_ticks) as usize,
    }));
    let life = task.life();
    drop(task);
    match block(
        WakeKey::Alarm { task: me },
        life,
        clock::ticks_to_duration(wait_ticks),
    )? {
        Handoff::Switch(pa) => Ok(Some(pa)),
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
}

pub fn wait<E: WaitFail>(key: WakeKey, life: Weak<Life>, dur: Duration) -> Result<Handoff<()>, E> {
    block(key, life, dur)
}

pub fn fall<E: WaitFail>(me: TaskLife, dur: Duration) -> Result<Handoff<bool>, E> {
    let TaskLife { id, life } = me;
    match block(WakeKey::Pies { task: id }, life, dur)? {
        Handoff::Switch(pa) => Ok(Handoff::Switch(pa)),
        Handoff::Resume(()) => Ok(Handoff::Resume(true)),
    }
}

pub(crate) fn wipe(key: WakeKey) -> usize {
    let _commit = crate::work::unit::commit();
    let chain = {
        let mut sites = sites(key).lock();
        sites.remove(&key)
    };
    let chain = match chain {
        Some(site) => {
            for (id, life) in site.fwd.entries() {
                knock(WakeKey::Tole { id }, life);
            }
            site.head
        }
        None => None,
    };
    rise(Unchain { cur: chain })
}

pub(crate) fn knock(key: WakeKey, life: &Weak<Life>) -> usize {
    let _commit = crate::work::unit::commit();
    let mut fwd = Fwd::empty();
    let chain = {
        let mut sites = sites(key).lock();
        let chain = match sites.get_mut(&key) {
            Some(site) => {
                fwd = site.fwd.clone();
                let chain = site.head.take(); site.tail = None;
                if chain.is_none() { site.pend = true; }
                chain
            }
            None => {
                if !Life::dead(life) && sites.try_reserve(1).is_ok() {
                    let mut site = Site::new(life); site.pend = true; sites.insert(key, site);
                }
                None
            }
        };
        prune(&mut sites, key); chain
    };
    for (id, life) in fwd.entries() { knock(WakeKey::Tole { id }, life); }
    rise(Unchain { cur: chain })
}

pub(crate) fn forward(
    key: WakeKey,
    life: Weak<Life>,
    tole: usize,
    tole_life: Weak<Life>,
) -> Result<(), ()> {
    let mut sites = sites(key).lock();
    if !sites.contains_key(&key) {
        sites.try_reserve(1).map_err(|_| ())?;
        sites.insert(key, Site::new(&life));
    }
    let site = sites.get_mut(&key).ok_or(())?;
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

/// **只投给已登记的观察者**。
///
/// 与另外两条的分界一句话成族：`wake` 交付**一个**等待者（顺带提示转发组）、`knock` 是
/// 提示型（整链放行，**允许就地建站点**）、本函数**只对已经站着的那个站点说话**：
/// 有转发边就逐条把它转成对观察组的 `knock`，有直等链就整链摘下来放行；
/// **站点不在 ⇒ 直接返回 0，不建站点、不分配、不置位**——没有观察者的键不留痕。
///
/// 那条"不留痕"是承重的：能力变化走的是造门闩那条热路径，若为每个改过表的任务都留下
/// 一个站点，站点表会随任务数单调长大。
///
/// **不写站点自己的 `pend`**：这个键的合法消费者只有经转发边挂上来的组——把提示留在
/// 本键上无人消费，反而会让站点永远过不了 [`prune`]。
///
/// 锁：`Fwd` 快照只在分片锁内取，出锁才 knock（与 `wake` 同形，不跨 shard 嵌套加锁）。
pub(crate) fn signal(key: WakeKey) -> usize {
    let _commit = crate::work::unit::commit();
    let mut fwd = Fwd::empty();
    let chain = {
        let mut sites = sites(key).lock();
        let chain = match sites.get_mut(&key) {
            Some(site) => {
                fwd = site.fwd.clone();
                let chain = site.head.take();
                site.tail = None;
                chain
            }
            None => None,
        };
        prune(&mut sites, key);
        chain
    };
    for (id, life) in fwd.entries() {
        knock(WakeKey::Tole { id }, life);
    }
    rise(Unchain { cur: chain })
}

/// 站点总数（只给健康面读：证"没有观察者就不建站点"这一条）。
#[cfg(debug_assertions)]
pub(crate) fn site_count() -> usize {
    (0..SITE_SHARDS)
        .map(|shard| shard_at(shard).lock().len())
        .sum()
}

pub(crate) fn wipe_space(space: Asid) -> usize {
    let _commit = crate::work::unit::commit();
    let mut woken = 0usize;
    for shard in 0..SITE_SHARDS {
        loop {
            let taken = {
                let mut sites = shard_at(shard).lock();
                let key = sites
                    .keys()
                    .find(|k| matches!(k, WakeKey::Space { space: s, .. } if *s == space))
                    .copied();
                key.and_then(|key| sites.remove(&key))
            };
            let Some(site) = taken else { break };
            woken += rise(Unchain { cur: site.head });
        }
    }
    woken
}

pub fn wake(key: WakeKey, life: &Weak<Life>) -> bool {
    let _commit = crate::work::unit::commit();
    debug_assert!(
        !matches!(key, WakeKey::Tole { .. }),
        "组键要走 knock（提示型/整链放行），不能走 wake（交付型/一人）"
    );
    let mut fwd = Fwd::empty();
    let popped = {
        let mut sites = sites(key).lock();
        if Life::dead(life) {
            let chain = sites.remove(&key).and_then(|site| site.head);
            drop(sites);
            rise(Unchain { cur: chain });
            return false;
        } else {
            let mut beacon_only = false;
            let popped = match sites.get_mut(&key) {
                Some(site) => match site.pop_front() {
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
            if beacon_only && sites.try_reserve(1).is_ok() {
                let mut site = Site::new(life);
                site.pend = true;
                sites.insert(key, site);
            }
            fwd = sites.get(&key).map_or_else(Fwd::empty, |s| s.fwd.clone());
            prune(&mut sites, key);
            popped
        }
    };
    for (id, life) in fwd.entries() {
        knock(WakeKey::Tole { id }, life);
    }
    let Some(task) = popped else { return false };
        void(Task::blocked_ticket(&task));
    rise(core::iter::once(task));
    true
}

pub fn redeem() -> bool {
    let _commit = crate::work::unit::commit();
    const MAX_DUE: usize = 64;
    let mut due = [0u64; MAX_DUE];
    let n = timer::drain(clock::now(), &mut due);
    let mut tasks: [Option<Arc<Task>>; MAX_DUE] = [const { None }; MAX_DUE];
    let mut woken = 0usize;
    for (slot, &handle) in tasks.iter_mut().zip(&due[..n]) {
        let Some((key, holder)) = void(Ticket(handle)) else {
            continue;
        };
        drop(holder);
        let popped = {
            let mut sites = sites(key).lock();
            let pick = &mut |t: &mut Arc<Task>| Task::blocked_ticket(t) == Ticket(handle);
            let w = sites.get_mut(&key).and_then(|site| site.remove_if(pick));
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

/// Keep the request and one deadline in Blocked, so a wakeup never becomes a
/// false completion and observers pin the original shared record until return.
pub(crate) fn unit_join(join: crate::work::unit::join::JoinWait)
    -> Result<Handoff<env::JoinReply>, env::UnitFail> {
    loop {
        let _commit = crate::work::unit::commit();
        let reply = join.poll()?;
        if reply.is_reaped() || join.remaining() == Duration::ZERO { return Ok(Handoff::Resume(reply)); }
        let blocked = block_with_call::<env::UnitFail>(join.key(), join.life(), join.remaining(), Some(WaitCall::Join(join.clone())))?;
        // Registration is atomic with the poll. Scheduler idle waits must run
        // after releasing Unit, so another hart can publish or wake a task.
        drop(_commit);
        match finish_block(blocked) {
            Handoff::Resume(()) => continue,
            Handoff::Switch(pa) => return Ok(Handoff::Switch(pa)),
        }
    }
}
fn resume_join(task: &Arc<Task>, join: crate::work::unit::join::JoinWait) -> bool {
    use env::JoinReply;
    use crate::runtime::switcher::context::TrapContext;
    let _commit = crate::work::unit::commit();
    let result = join.poll();
    let result = if matches!(result, Ok(JoinReply::Pending)) && join.remaining() != Duration::ZERO {
        match requeue_join(task, join.clone()) {
            Ok(()) => return false,
            Err(error) => Err(error),
        }
    } else { result };
    // SAFETY: the detached blocked task is not executing; its frame remains owned.
    let frame = unsafe { &mut *(task.ident.frame.pa.expect("blocked frame").as_usize() as *mut TrapContext) };
    crate::work::unit::join::JoinWait::write(frame, result);
    true
}
fn requeue_join(task: &Arc<Task>, join: crate::work::unit::join::JoinWait) -> Result<(), env::UnitFail> {
    let key = join.key(); let life = join.life(); let dur = join.remaining();
    {
        let mut table = sites(key).lock();
        if !table.contains_key(&key) {
            table.try_reserve(1).map_err(|_| env::UnitFail::OoM)?;
            table.insert(key, Site::new(&life));
        }
    }
    let ticket = Ticket::alloc();
    if dur != Duration::MAX {
        hold(ticket, key, task).map_err(|_| env::UnitFail::OoM)?;
        if timer::tock(ticket.raw(), clock::now().add(dur).as_ticks()).is_err() {
            void(ticket); return Err(env::UnitFail::OoM);
        }
    }
    let stopped = task.stopped();
    let state = if stopped { TaskState::Debarked { state: crate::work::unit::task::TaskStopped::Blocked {
        key, ticket, next: None, wait: Some(WaitCall::Join(join)),
    } } } else { TaskState::Blocked { key, ticket, next: None, wait: Some(WaitCall::Join(join)) } };
    *task.state.lock() = state;
    let mut table = sites(key).lock();
    let site = table.get_mut(&key).expect("join site"); site.pend = false; site.push_back(task.clone());
    Ok(())
}

#[derive(Clone)]
pub(crate) enum WaitCall { Join(crate::work::unit::join::JoinWait), Mail(crate::work::mail::tole::AwaitWait) }
pub(crate) fn mail_await(wait: crate::work::mail::tole::AwaitWait) -> Result<Handoff<env::AwaitReply>, env::MailFail> {
    loop {
        let commit = crate::work::unit::commit();
        let reply = match wait.poll() { Ok(reply) => reply, Err(e) => { wait.unwatch(); return Err(e) } };
        if reply != env::AwaitReply::Pending || wait.remaining() == Duration::ZERO { wait.unwatch(); return Ok(Handoff::Resume(reply)); }
        wait.watch()?;
        let blocked = match block_with_call::<env::MailFail>(wait.key(), wait.life(), wait.remaining(), Some(WaitCall::Mail(wait.clone()))) { Ok(blocked) => blocked, Err(e) => { wait.unwatch(); return Err(e) } };
        drop(commit);
        match finish_block(blocked) { Handoff::Resume(()) => continue, Handoff::Switch(pa) => return Ok(Handoff::Switch(pa)) }
    }
}
fn resume_await(task: &Arc<Task>, wait: crate::work::mail::tole::AwaitWait) -> bool {
    let _commit = crate::work::unit::commit();
    let result = wait.poll();
    let result = if matches!(result, Ok(env::AwaitReply::Pending)) && wait.remaining() != Duration::ZERO {
        match requeue_await(task, wait.clone()) { Ok(()) => return false, Err(e) => Err(e) }
    } else { result };
    // SAFETY: this detached blocked task owns a stable frame and is not executing.
    let frame = unsafe { &mut *(task.ident.frame.pa.expect("blocked frame").as_usize() as *mut crate::runtime::switcher::context::TrapContext) };
    wait.unwatch(); crate::work::mail::tole::AwaitWait::write(frame, result); true
}
fn requeue_await(task: &Arc<Task>, wait: crate::work::mail::tole::AwaitWait) -> Result<(), env::MailFail> {
    let key = wait.key(); let life = wait.life(); let dur = wait.remaining();
    {
        let mut table = sites(key).lock();
        if !table.contains_key(&key) { table.try_reserve(1).map_err(|_| env::MailFail::OoM)?; table.insert(key, Site::new(&life)); }
    }
    let ticket = Ticket::alloc();
    if dur != Duration::MAX {
        hold(ticket, key, task).map_err(|_| env::MailFail::OoM)?;
        if timer::tock(ticket.raw(), clock::now().add(dur).as_ticks()).is_err() { void(ticket); return Err(env::MailFail::OoM); }
    }
    let state = if task.stopped() { TaskState::Debarked { state: crate::work::unit::task::TaskStopped::Blocked { key, ticket, next: None, wait: Some(WaitCall::Mail(wait)) } } }
        else { TaskState::Blocked { key, ticket, next: None, wait: Some(WaitCall::Mail(wait)) } };
    *task.state.lock() = state;
    let mut table = sites(key).lock(); let site = table.get_mut(&key).expect("await site"); site.pend = false; site.push_back(task.clone()); Ok(())
}
