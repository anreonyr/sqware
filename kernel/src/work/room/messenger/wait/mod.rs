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

impl WaitFail for env::ToleFail {
    fn busy() -> Self {
        env::ToleFail::Busy
    }
    fn oom() -> Self {
        env::ToleFail::OoM
    }
}

fn block<E: WaitFail>(key: WakeKey, life: Weak<Life>, dur: Duration) -> Result<Handoff<()>, E> {
    if take_beacon(key) {
        return Ok(Handoff::Resume(()));
    }
    let Some(me) = current().running_task() else {
        return Err(E::busy());
    };
    if let Some(reason) = super::take_doomed(me.ident.id) {
        drop(me);
        super::set_exit_reason(reason);
        return Ok(Handoff::Switch(super::quit()));
    }
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
    let (mut task, next_pa) = current().swap();
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id.get(),
        key: key.fold() as usize,
    }));
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        key,
        ticket,
        next: None,
    });
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
    Ok(Handoff::Switch(next_pa.unwrap_or_else(run)))
}

fn rise<I: IntoIterator<Item = Arc<Task>>>(tasks: I) -> usize {
    let mut woke = 0;
    for task in tasks {
        let mut t = task;
        Task::exclusive(&mut t).transform(TaskState::Starved { next: None });
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
        void(Task::blocked_ticket(&mut task));
        self.cur = Task::blocked_next(&mut task).take();
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

pub fn join<E: WaitFail>(task: TaskLife, reaped: bool, dur: Duration) -> Result<Handoff<bool>, E> {
    if reaped {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    let TaskLife { id, life } = task;
    match block(WakeKey::Task { id }, life, dur)? {
        Handoff::Switch(pa) => Ok(Handoff::Switch(pa)),
        Handoff::Resume(()) => Ok(Handoff::Resume(true)),
    }
}

pub(crate) fn wipe(key: WakeKey) -> usize {
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
    let chain = {
        let mut sites = sites(key).lock();
        let chain = match sites.get_mut(&key) {
            Some(site) => {
                let chain = site.head.take();
                site.tail = None;
                if chain.is_none() {
                    site.pend = true;
                }
                chain
            }
            None => {
                if !Life::dead(life) && sites.try_reserve(1).is_ok() {
                    let mut site = Site::new(life);
                    site.pend = true;
                    sites.insert(key, site);
                }
                None
            }
        };
        prune(&mut sites, key);
        chain
    };
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

pub(crate) fn wipe_space(space: Asid) -> usize {
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
    debug_assert!(
        !matches!(key, WakeKey::Tole { .. }),
        "组键要走 knock（提示型/整链放行），不能走 wake（交付型/一人）"
    );
    let mut fwd = Fwd::empty();
    let popped = {
        let mut sites = sites(key).lock();
        if Life::dead(life) {
            sites.remove(&key);
            None
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
    let Some(mut task) = popped else { return false };
    void(Task::blocked_ticket(&mut task));
    rise(core::iter::once(task));
    true
}

pub fn redeem() -> bool {
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
