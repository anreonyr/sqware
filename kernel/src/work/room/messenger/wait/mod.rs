pub(super) mod holder;
pub(super) mod site;

use core::sync::atomic::{AtomicUsize, Ordering};
use alloc::sync::{Arc, Weak};
use core::time::Duration;

use crate::memory::manager::asid::Asid;
use env::HoleDir;
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
        kicklog::note(t.ident.id.get());
        kick(conductor::pick(), t);
        woke += 1;
    }
    woke
}

/// **（临时诊断）"踢出去"与"真跑起来"之间的那一格**：`rise`（唤醒唯一那一手）记一笔时刻，
/// 任务下一次进内核（`envcall/tole.rs::await_` 开头）对号读回来。
///
/// 为什么要有它：`wake`/`knock` 把车从站点上摘下来之后，到它真的在 hart 上跑起来之间还有一段
/// 完全看不见的路（`kick` → 运行队列 → 调度挑中 → 切上下文）。一个组等了几秒才回话，
/// **"这一敲根本没来"与"来了、可它几秒后才被跑"是两种病**，只有这一格分得开。
///
/// 只留 8 格环形（够对号用），全是 `Relaxed` 原子：**热路上不加锁**。
pub(crate) mod kicklog {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use crate::runtime::chrono::clock;

    const N: usize = 8;
    static ID: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];
    static AT: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// 记一笔"刚把这一位踢上运行队列"。
    pub(crate) fn note(id: usize) {
        if id == 0 {
            return;
        }
        let i = NEXT.fetch_add(1, Ordering::Relaxed) % N;
        AT[i].store(clock::uptime_ticks() as usize, Ordering::Relaxed);
        ID[i].store(id, Ordering::Relaxed);
    }

    /// 这一位最近一次被踢之后过了多少毫秒（没记到 ⇒ `None`）。
    pub(crate) fn lag_ms(id: usize) -> Option<usize> {
        let now = clock::uptime_ticks();
        let mut best: Option<usize> = None;
        for i in 0..N {
            if ID[i].load(Ordering::Relaxed) != id {
                continue;
            }
            let at = AT[i].load(Ordering::Relaxed) as u64;
            if at == 0 {
                continue;
            }
            let ms = clock::ticks_to_duration(now.wrapping_sub(at)).as_millis() as usize;
            if best.map_or(true, |b| ms < b) {
                best = Some(ms);
            }
        }
        best
    }
}

pub(crate) use kicklog::lag_ms as kick_lag;

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
            // **（临时读数）这一格是被"抹"掉的**（孔封印/组收场）：`fwd` 是**抹掉时**还挂着的
            // 组数——被抹的那一枚若还有人挂，那几位从此再也收不到这一格的唤醒。**带挂的必打**
            // （封顶 200），空的那种只打前 40 次。
            let n_fwd = site.fwd.entries().count();
            let said = if n_fwd > 0 {
                static HOT: AtomicUsize = AtomicUsize::new(0);
                HOT.fetch_add(1, Ordering::Relaxed) < 200
            } else {
                static COLD: AtomicUsize = AtomicUsize::new(0);
                COLD.fetch_add(1, Ordering::Relaxed) < 40
            };
            if said {
                let (id, kind) = site::label(key);
                crate::putln!(
                    "site: wipe key={} kind={} fwd={} head={}",
                    id,
                    kind,
                    n_fwd,
                    site.head.is_some()
                );
            }
            for (id, life) in site.fwd.entries() {
                knock(WakeKey::Tole { id }, life);
            }
            site.head
        }
        None => None,
    };
    rise(Unchain { cur: chain })
}

/// **（临时读数）组那一侧的敲落点**：`KNOCK_N` = 敲了几次，`KNOCK_POP` = 其中**摘下了车**的几次。
///
/// 为什么要它：`ring`/`give` 报的那一行要与"这一敲到底算不算数"分开——**`pop` 为 0** ⇒
/// 目标组当时**不在这一格上停着**（站点不在/空着 ⇒ 只立了一枚位），`pop` 为 1 ⇒ 车摘下来、
/// 已 `kick`（此后若还没跑，账就在调度那一侧）。读数由**摇的一方**当场读（被敲的一方若再也
/// 不跑，它自己报不了），故这两格必须是全局计数。
static KNOCK_N: AtomicUsize = AtomicUsize::new(0);
static KNOCK_POP: AtomicUsize = AtomicUsize::new(0);

/// 读这两格（取差值用）。
pub(crate) fn knock_stats() -> (usize, usize) {
    (
        KNOCK_N.load(Ordering::Relaxed),
        KNOCK_POP.load(Ordering::Relaxed),
    )
}

pub(crate) fn knock(key: WakeKey, life: &Weak<Life>) -> usize {
    KNOCK_N.fetch_add(1, Ordering::Relaxed);
    let (chain, popped) = {
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
                // **（临时读数）这一敲**被丢**了没有**：站点不存在时，只有"这个组还活着 ＋ 备得下
                // 一格"才立得出站点（`pend=true` ⇒ 那一位下次进门当场就复扫一遍）。两条都不成立
                // ⇒ 这一敲**静默消失**——"该叫没叫"最硬的一条原文，故第一次就报一行。
                let dead = Life::dead(life);
                if !dead && sites.try_reserve(1).is_ok() {
                    let mut site = Site::new(life);
                    site.pend = true;
                    sites.insert(key, site);
                } else {
                    static N: AtomicUsize = AtomicUsize::new(0);
                    let n = N.fetch_add(1, Ordering::Relaxed);
                    if n < 20 {
                        crate::putln!(
                            "knock dropped key={} dead={} (no site, no reserve)",
                            key.fold(),
                            dead
                        );
                    }
                }
                None
            }
        };
        let popped = chain.is_some();
        prune(&mut sites, key);
        (chain, popped)
    };
    if popped {
        KNOCK_POP.fetch_add(1, Ordering::Relaxed);
    }
    let woken = rise(Unchain { cur: chain });
    // **（临时读数）组这一侧到底醒了几个人**：0 = "knock 到了，可这一组没有停着的车"。
    if let WakeKey::Tole { id } = key {
        static N: AtomicUsize = AtomicUsize::new(0);
        if N.fetch_add(1, Ordering::Relaxed) < 40 {
            crate::putln!("knock tole#{} woken={}", id, woken);
        }
    }
    woken
}

pub(crate) fn forward(
    key: WakeKey,
    life: Weak<Life>,
    tole: usize,
    tole_life: Weak<Life>,
) -> Result<(), ()> {
    let mut sites = sites(key).lock();
    let fresh = !sites.contains_key(&key);
    let was = sites.get(&key).map_or(0, |s| s.fwd.entries().count());
    if fresh {
        sites.try_reserve(1).map_err(|_| ())?;
        sites.insert(key, Site::new(&life));
    }
    let site = sites.get_mut(&key).ok_or(())?;
    let r = site.fwd.attach(tole, tole_life);
    // **（临时读数）哪一组挂上了哪一格**：`fresh=1` = 这一格**刚从空站点立起来**（站点被摘过
    // 之后又来挂，就会看到这一格）——"转发是不是被摘过"在这一行上直接可判。`fresh` 或失败
    // **必打**（封顶 200），其余只打前 40 次。
    let said = if fresh || r.is_err() {
        static HOT: AtomicUsize = AtomicUsize::new(0);
        HOT.fetch_add(1, Ordering::Relaxed) < 200
    } else {
        static COLD: AtomicUsize = AtomicUsize::new(0);
        COLD.fetch_add(1, Ordering::Relaxed) < 40
    };
    if said {
        let (id, kind) = site::label(key);
        crate::putln!(
            "site: forward key={} kind={} tole={} fresh={} was={} now={} ok={}",
            id,
            kind,
            tole,
            fresh,
            was,
            site.fwd.entries().count(),
            r.is_ok()
        );
    }
    prune(&mut sites, key);
    r
}

pub(crate) fn unforward(key: WakeKey, tole: usize) {
    let mut sites = sites(key).lock();
    if let Some(site) = sites.get_mut(&key) {
        let before = site.fwd.entries().count();
        site.fwd.detach(tole);
        // **（临时读数）哪一格把**哪一组**摘了、摘完还剩几组**：`left=0` 就是"这一格的转发空了"
        // ——此后这一格上的摇再也敲不到任何组（那个组只在**又一次 attach** 时才会回来）。
        // `was>0`（真摘掉了一条转发）**必打**、封顶 200；其余只打前 40 次。
        let left = site.fwd.entries().count();
        let said = if before > 0 {
            static HOT: AtomicUsize = AtomicUsize::new(0);
            HOT.fetch_add(1, Ordering::Relaxed) < 200
        } else {
            static COLD: AtomicUsize = AtomicUsize::new(0);
            COLD.fetch_add(1, Ordering::Relaxed) < 40
        };
        if said {
            let (id, kind) = site::label(key);
            crate::putln!(
                "site: unforward key={} kind={} tole={} was={} left={}",
                id,
                kind,
                tole,
                before,
                left
            );
        }
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
    // **（临时读数）这一摇带没带转发 / 叫醒了谁**：
    // · `dir=Pull/Push` —— 两个方向**是两个站点**（`WakeKey::Hole{dir}`），故"投递摇的那一枚"
    //   与"取空摇的那一枚"在这一行上分得开（`give` 摇 Pull、`take`/`taken` 摇 Push）；
    // · `fwd != 0` —— 这一格有组在等，下面那圈 knock 会发出去；`first=` 是**第一个组号**
    //   （与 `unseal tole#… owner=…` 对号 ⇒ "叫的是不是那一位"当场可判）；
    // · `popped=true` —— 这一摇**直接**叫醒了一个停在这一格上的车（收信那一侧的关键事件）。
    //
    // 只打"有组在等"与"真叫醒了人"两种（一场装配里推入几百次，按条数封顶会把关键那几次吃掉）。
    {
        let count = fwd.entries().count();
        if count > 0 || popped.is_some() {
            let (hole, dir) = match key {
                WakeKey::Hole { hole, dir } => (hole, dir),
                _ => (usize::MAX, HoleDir::Pull),
            };
            let first = fwd.entries().next().map(|(id, _)| id).unwrap_or(0);
            crate::putln!(
                "wake hole#{} dir={:?} popped={} fwd={} first={}",
                hole,
                dir,
                popped.is_some(),
                count,
                first
            );
        }
    }
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
