// 事件队列（messenger）— 任务不在 running 槽时的状态机。
//
// 任务离开 running 槽有三种过渡：park（纯睡）、wait（按唤醒源等信号）、reap（退出）。
// 前两种与「等目标回收」现在共用一条挂起实现 [`block`]——它们只差一个键。
// 三种都借 [`scheduler::core::Scheduler::disown_and_install_next`] 跨边界原语把
// running 卸下（槽位 settled：装下一 starved 或降级 Last），再挂进站点表。
// 恢复路径三条：[`wake`]（信号到，一个）、[`wipe`]（键退役，全部）、
// [`redeem`]（到点，按票），都把任务转 Starved 推回本核 + kick。
//
// 簿记：sites（唤醒源 → 信标 + 等待者队列）、holders（票 → 持票人）、husks
// （Arc<Task> 队列）。**两张等待表分工**：站点表是挂起任务的唯一容器（谁在等、
// 等什么）；票根只回答「这一票是谁」（到点时凭票认领）。「等什么」不复制进票根
// ——从持票人自己那张票上读（`TaskState::Blocked`）。旧版的 parked /
// wait_times / join_times 三张旁路表因此消失。
// 锁序：**L1（调度器）与 L3（本域各表）任何方向都不得嵌套**——持任一 L3 期间
// 不得调用 scheduler 的任何加锁方法，也不得在锁内 drop `Arc<Task>`（drop 链会
// 取 Space 锁 L2）。各路径的写法统一为「作用域内取、作用域外用」：block/wake/
// wipe/redeem 在块内摘出 Waiter、块外 push 回 scheduler；bury 块内出队、
// 块外回收；rip 块内 take 整表、块外 drop。
//
// 反向耦合清零：dock / ring 的 task_exit 反向耦合走两步拆——step 5 引入 exit
// hook 注册面后，bury 不再硬编码子系统名。本 step 暂留直调作为过渡。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use env::HoleDir;
use hashbrown::{HashMap, HashSet};

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::{current, lookup_task_by_id};
use crate::work::room::scheduler::trap::run;
use crate::work::unit::gate::GateError;
use crate::work::unit::task::{Task, TaskState};
use crate::work::unit::team::Team;

// ── 票与票根 ──

/// 票：一次挂起的唯一标识。单调、不复用。
///
/// 到点登记（`timer::tock`）只携带它。凭它可以还原出「谁」——`HOLDERS` 拿着票号
/// 找持票人；而「等什么」在持票人自己那张票上（`TaskState::Blocked { key, ticket }`）。
/// 于是「到点了该叫醒谁」不再需要任何旁路表。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

impl Ticket {
    /// 发票。Relaxed 足够：票号只用于相等判定，不承载顺序。
    fn alloc() -> Ticket {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Ticket(NEXT.fetch_add(1, Ordering::Relaxed) as u64)
    }

    /// 到点登记用的裸值。
    fn raw(self) -> u64 {
        self.0
    }
}

/// 票根：票 → 持票人。**只存 `Weak`**。
///
/// 挂起任务的强持有者只能是它所在的站点队列（见模块头的「唯一强持有」）。这里若
/// 存 `Arc`，任务就有了第二个强持有者：一撞 `Task::exclusive` 的唯一性前提，二让
/// 陈旧的到点登记把已回收的任务钉住（关机审计会把它报成帧泄漏）。
fn holders() -> &'static SpinLock<HashMap<Ticket, Weak<Task>>> {
    static HOLDERS: OnceLock<SpinLock<HashMap<Ticket, Weak<Task>>>> = OnceLock::new();
    HOLDERS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 存根。**前置：到点登记尚未发生**——「堆可见 ⇒ 票根必在」，否则到期路径会命中
/// 一个空的票根。
fn hold(ticket: Ticket, task: &Arc<Task>) {
    holders().lock().insert(ticket, Arc::downgrade(task));
}

/// 作废票根并取回持票人：到期认领与提前作废走同一条路，**幂等**（票号不复用，
/// 第二次必得 `None`）。顺带消音它的到点——`timer::mute` 对已取走的句柄是 no-op，
/// 故到期路径重复调用也无害（代价是一次空扫，n = 未到点 tock 数）。
fn void(ticket: Ticket) -> Option<Arc<Task>> {
    timer::mute(ticket.raw());
    holders().lock().remove(&ticket).and_then(|w| w.upgrade())
}

// ── 类型 ──

/// 唤醒源：谁会把等待者叫醒。三个命名空间各占一个变体，键即身份。
///
/// **没有位打包**：`Space` 的两个字段各自完整，不再把 asid 挤进高 16 位、用户键
/// 截到低 48 位。旧 `WaitKey::compose` 的单射性靠掩码保证，还因此逼出一个
/// `#[inline(never)]` 的 mask helper 去躲 size 优化下的错联（§13.10 A）——枚举下
/// 这两样都不需要：没有 mask，就没有 mask 错联。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WakeKey {
    /// 调用方命名空间里的裸整数（`RoomCall::Wait` / `Wake`）：空间身份 + 槽位。
    Space { space: usize, slot: usize },
    /// 资源就绪（`MailCall::Wait`；hole 的 push / pull / seal 投信）。
    ///
    /// `hole` 用裸整数而非 `mail::HoleId`：依赖方向必须保持 mail → room 单向，
    /// 引 `HoleId` 就成了环。
    Hole { hole: usize, dir: HoleDir },
    /// 目标任务回收（`UnitCall::Join`）。
    Task { id: usize },
    /// 无人投信——只有期限会响（`RoomCall::Park`）。
    ///
    /// 键就是那个睡眠者本人：park 没有信号源，能唤醒它的只有它自己那次到点登记。
    Alarm { task: usize },
}

impl WakeKey {
    /// 折成 64 位——**只供分片**，不承载语义（相等性仍由 `Eq` 判定）。
    fn fold(self) -> u64 {
        match self {
            WakeKey::Space { space, slot } => {
                (space as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ slot as u64
            }
            WakeKey::Hole { hole, dir } => {
                (hole as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ dir as u64
            }
            WakeKey::Task { id } => (id as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
            WakeKey::Alarm { task } => (task as u64).wrapping_mul(0xA24B_AED4_963E_E407),
        }
    }
}

/// 一个唤醒源的等待位：遗留信号（信标）+ 等待者队列。
///
/// 三种唤醒源共用本类型（旧版 `WaitSite` / `JoinSite` 字段逐个相同——各自一份是
/// 键的 Rust 类型不同逼出来的）。
struct Site {
    /// 遗留信号（信标）：wake 无等待者 → 置位；wait 见位 → 消费即回（防漏唤醒）。
    pend: bool,
    /// 等待者（FIFO）；每项携带到点句柄（无期限 = None）。
    waiters: VecDeque<Waiter>,
}

/// 落点：一次可能离核的调用之后，由谁占住处理器。
///
/// `Resume(T)` 是**没离核**（当前帧由调用方持有——本域不知道也不该知道当前帧是
/// 什么），`T` 是该次调用的**当场结论**；`Switch(pa)` 是**已离核**。
///
/// 两态而非三态：本核无后继不再是一种落点——那是「取活」，由本域内部 `run()`
/// 收口，不是调用方该知道的事（旧 `Idle` 把这个收尾漏给了两个不同层的调用方）。
/// 于是 `Handoff` / `Joined` / `JoinStep` / `Waited` 四种拼写收成一个。
pub enum Handoff<T> {
    /// 未离核：继续跑调用方的当前帧；`T` = 当场结论。
    Resume(T),
    /// 已离核：切到该帧（本核 starved 队首已装槽，或本域取活取来）。
    Switch(usize),
}

/// 等待者：站点队列里的一项。票号即「哪一次挂起」——同一任务先后等同一个键时，
/// 靠它区分，故陈旧的到点登记不可能偷走后来的那次等待。
struct Waiter {
    task: Arc<Task>,
    ticket: Ticket,
}

// ── 簿记表（全部 L3） ──

/// 事件等待表的分片数。每片 = 一把 L3 锁 + 一个 HashMap；wait/wake/drain
/// 按 [`site_shard`] 纯函数路由到同片，跨片互不阻塞——把单点串行竞争降到
/// 1/SITE_SHARDS（典型 16）。分片数取 2 的幂：位与替代 mod。
const SITE_SHARDS: usize = 16;
const SITE_SHARDS_MASK: usize = SITE_SHARDS - 1;

/// 唤醒源 → 分片（pure function，所有路径一致：wait / wake / 投信 / 到期都经此）。
/// splitmix64 折叠 64→32 后按位与 SHARDS 掩码——高位低位的熵都被采样。
#[inline]
fn site_shard(key: WakeKey) -> usize {
    let h = key.fold().wrapping_mul(0x9E3779B97F4A7C15);
    ((h >> 32) ^ h) as usize & SITE_SHARDS_MASK
}

/// 站点表（Level::L3，绝不 3→3 嵌套）。**分片版**：每片一把
/// L3 锁 + HashMap，单一线性化点缩小到一片——wait / wake 跨片并行。
///
/// **一张表装三种唤醒源**：它们的等待者是同一种东西（任务 + 到点句柄），唤醒
/// 也是同一件事（摘出 → 放回就绪）。旧版把「等目标回收」单独放进 `joins`，理由
/// 只是键的 Rust 类型不同（`usize` vs `WaitKey`）——键成枚举之后，那个理由没了。
///
/// 锁纪律：仍是 L3、可与 timer 锁共存但**绝不 3→3 嵌套**（rip 路径循环逐片清，
/// 禁持跨片锁）。同 key 的所有 waiters 必落在同一分片（`site_shard` 纯函数保证），
/// 唤醒不必跨片扫描。
fn shard_at(shard: usize) -> &'static SpinLock<HashMap<WakeKey, Site>> {
    static SHARDS: OnceLock<Box<[SpinLock<HashMap<WakeKey, Site>>]>> = OnceLock::new();
    let arr: &'static [SpinLock<HashMap<WakeKey, Site>>] = SHARDS.get_or_init(|| {
        let mut v: Vec<SpinLock<HashMap<WakeKey, Site>>> = Vec::with_capacity(SITE_SHARDS);
        for _ in 0..SITE_SHARDS {
            v.push(SpinLock::new_level(Level::L3, HashMap::new()));
        }
        v.into_boxed_slice()
    });
    &arr[shard]
}

/// 本唤醒源的站点表分片。
fn sites(key: WakeKey) -> &'static SpinLock<HashMap<WakeKey, Site>> {
    shard_at(site_shard(key))
}

/// 全局躯壳队列（Level::L3，与 Team.tasks 同级）：延迟回收——不能在
/// 自己正在用的栈上回收自己；bury 统一回收。
pub(super) static HUSKS: SpinLock<VecDeque<Arc<Task>>> =
    SpinLock::new_level(Level::L3, VecDeque::new());

/// 待杀集合（doomed）：`kill` 点名他核 Running 任务时记入，目标核 trap 自查
/// 自退。无主簿记——只存 task_id，不持 `Arc<Task>`（防「杀者撑着被杀者」）。
/// Level::L3，与站点表同级。
fn doomed() -> &'static SpinLock<HashSet<usize>> {
    static T: OnceLock<SpinLock<HashSet<usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashSet::new()))
}

// ── 操作：挂起（用 scheduler::core::Scheduler::disown_and_install_next） ──

/// 挂起的唯一实现：三处入口（`park` / `wait` / `join`）只差一个键。
///
/// 时序（两个竞态闭合点）：
///   ① 信标先探——信号已至 → 不挂起（不碰站点表：缺键即无信标）
///   ② 离核——借 scheduler 跨边界原语把 running 卸下（槽位 settled）
///   ③ 登记——发票 → 存票根 → `tock`（**先票根后 tock**：堆可见 ⇒ 票根必在）
///   ④ 入队——写等待点 + 挂进站点队列；**锁内再查一次信标**
///   ⑤ 窗口内信号已至 → 撤销登记，按「已唤醒」处理（Starved 入队）
///
/// 锁纪律：站点表与票根都是 L3，**绝不互相嵌套**——「作用域内取、作用域外用」。
fn block(key: WakeKey, dur: Duration) -> Handoff<()> {
    // ① 信标先探
    if take_beacon(key) {
        return Handoff::Resume(());
    }
    // ② 离核
    let (mut task, next_pa) = current().disown_and_install_next();
    // ③ 登记
    let ticket = Ticket::alloc();
    let at = (dur != Duration::MAX).then(|| clock::now().add(dur).as_ticks());
    if let Some(at) = at {
        hold(ticket, &task);
        timer::tock(ticket.raw(), at);
    }
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id,
        // 诊断用折叠值：键成枚举后不再有「人可读的位打包」形态。
        key: key.fold() as usize,
    }));
    // ④ 写等待点 + 入队（锁内查信标）
    Task::exclusive(&mut task).transform(TaskState::Blocked { key, ticket });
    let queued = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(Site::new);
        let queued = if site.pend {
            site.pend = false;
            false
        } else {
            site.waiters.push_back(Waiter {
                task: task.clone(),
                ticket,
            });
            true
        };
        // 撤销阻塞那一支没有留下等待者：空壳站点随手删掉。
        prune(&mut sites, key);
        queued
    };
    // ⑤ 窗口内信标已至：撤销登记，按已唤醒处理
    if !queued {
        void(ticket);
        rise(core::iter::once(task));
    }
    // 本核无后继即就地取活：`run()` 只会循环到有帧或停机，故落点恒为 `Switch`。
    Handoff::Switch(next_pa.unwrap_or_else(run))
}

/// 放回就绪——「唤醒」的全部效果就是这一件事。
///
/// `wake` / `wipe` / `redeem` 与撤销阻塞四条路径的收尾完全同形（置 Starved →
/// 记事件 → 推回本核 starved 队列），故只写一遍。`kick` 提到批量之后：push 先于
/// 踢，唤醒方进入 steal 必可见（单 tick 的 IPI 量从 O(N) → O(1)）。返回唤醒数。
fn rise<I: IntoIterator<Item = Arc<Task>>>(tasks: I) -> usize {
    let mut woke = 0;
    for task in tasks {
        let mut t = task;
        Task::exclusive(&mut t).transform(TaskState::Starved);
        trace::note(EventKind::Room(RoomEvent::Wake { tid: t.ident.id }));
        current().push(t);
        woke += 1;
    }
    if woke > 0 {
        conductor::kick();
    }
    woke
}

/// 信标先探：消费本键上的遗留信号。**缺键即无信标**——不 `or_insert`：空的、
/// 无信标的站点没有语义，不该被「先探」凭空造出来。
fn take_beacon(key: WakeKey) -> bool {
    let mut sites = sites(key).lock();
    match sites.get_mut(&key) {
        Some(site) if site.pend => {
            site.pend = false;
            true
        }
        _ => false,
    }
}

/// 站点存在的判据：**队列非空 ∨ 有信标**。出队之后若不成立即删——空壳站点没有
/// 语义，留着就是 A2 那条「站点永不回收」的老毛病（`park` 每次睡眠都会留一个）。
/// 前置：已持有该分片的锁。
fn prune(sites: &mut HashMap<WakeKey, Site>, key: WakeKey) {
    if let Some(site) = sites.get(&key)
        && site.waiters.is_empty()
        && !site.pend
    {
        sites.remove(&key);
    }
}

/// 纯睡（`RoomCall::Park`）：键是 `Alarm { 我 }`——无人投信，只有期限会响。
///
/// Running → Blocked；返回下一帧 PA（若 scheduler 装了下一 starved）。
pub fn park(duration: Duration) -> usize {
    let me = current()
        .running_task()
        .expect("park: no running task")
        .ident
        .id;
    let wake_at = clock::now().add(duration).as_ticks();
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: me,
        wake_at: wake_at as usize,
    }));
    match block(WakeKey::Alarm { task: me }, duration) {
        Handoff::Switch(pa) => pa,
        // `Alarm` 无投信方 ⇒ 信标先探不可能命中。
        Handoff::Resume(()) => unreachable!("Alarm 无投信方"),
    }
}

/// 事件等待（`RoomCall::Wait`）：直通 [`block`]。有投信方的键，信标先探可能命中
/// 而当场续跑（[`Handoff::Resume`]）。
pub fn wait(key: WakeKey, dur: Duration) -> Handoff<()> {
    block(key, dur)
}

/// 死亡唯一入口：**收尾**（退出钩子：通道级联 + 能力级联）→ 置 `Reaped` → 入躯壳队列。
///
/// 不变量：`TaskState::Reaped` ⇔ 退出钩子已跑完——本函数是通往 `Reaped` 的**唯一**
/// 路径，也是躯壳队列的**唯一**入队点。`Join` 的判据 [`target_dead`] 因此精确：
/// 它返回真即「收尾已完成」。
///
/// 前置：任务已停摆（`Doomed`）；或从自退路径来（此刻已离核、状态仍是 `Running`，
/// 本函数就地补一次停摆）。已 `Reaped` 的直接返回。
///
/// 锁纪律：无锁调用。钩子只逐任务取放 L3（`Task.pies` / 通道注册表），且 [`cull`]
/// 已把整棵子树的受害者停摆在前——故钩子内再扑杀子域，也不会唤醒「还能跑」的人。
fn reap(mut task: Arc<Task>) {
    match task.state() {
        TaskState::Reaped => return,
        TaskState::Doomed => {}
        _ => Task::exclusive(&mut task).transform(TaskState::Doomed),
    }
    hooked(task.ident.id);
    Task::exclusive(&mut task).transform(TaskState::Reaped);
    HUSKS.lock().push_back(task); // L3 单独锁，1 → 3 顺序、不嵌套
}

/// quit：离核装槽 → [`reap`]（收尾 + 入队）→ 返下一帧 PA。
///
/// 延迟回收的理由是**回收**而非收尾：不能在自己正在用的栈上回收自己，故栈/trap
/// 帧/团队空间留到 `bury`；收尾（钩子）在此刻就做完了。
pub fn quit() -> Option<usize> {
    let cond = current();
    // 离核且无后继装槽 → 槽已 settled（disown_and_install_next 内 shed 或
    // 装下一）；团队 Arc 归零即回收——地址空间随释放。
    let (exited, next_pa) = cond.disown_and_install_next();
    debug_assert!(
        matches!(exited.state(), TaskState::Running { .. }),
        "running 容器里不是 Running 任务"
    );
    trace::note(EventKind::Room(RoomEvent::Exit {
        tid: exited.ident.id,
    }));
    reap(exited);
    // 注意：回收计数（conductor::exit）不在入队时递增——须等 bury 完成栈/
    // trap 帧/团队空间归还后再计数，否则最后任务退出时另一核见 REAPED==PUSHED
    // 立即 halt，本核 bury 未及回收 → 关机断言误报帧泄漏。
    next_pa
}

// ── 操作：等目标回收（Join） ──

/// 目标是否已死透。注册表只存 `Weak` 且从不清理：升级失败 ⇒ 已分配过就是
/// 「已回收」；从未分配 ⇒ 非法 id（调用方另判 `Denied`）。
///
/// `Reaped` 由 [`reap`] 独占置位（钩子之后），故本判据为真 ⇔ **收尾已完成**。
fn target_dead(tid: usize) -> bool {
    match crate::work::room::scheduler::core::lookup_task_by_id(tid) {
        Some(t) => t.state() == TaskState::Reaped,
        None => crate::work::unit::task::allocated(tid),
    }
}

/// 等目标结束（`Join` 的承载）。
///
/// 契约（与 `wait`/`pull` 同源）：**未挂起**时结论精确；**挂起过**则恢复后读到的
/// a0 是挂起前预置值（内核没有第二次执行机会），调用方须复探 `Join{task, 0}`。
/// 唤醒由**内核驱动**——目标收尾（含 fault isolation 杀）时 [`wipe`]
/// 放行全部等待者，故用户态跑不到的死亡也能被观察到。
///
/// 「结束」= 目标已死**且退出钩子（通道级联 + 能力级联）已跑完**——即返回真时，
/// 它名下的门闩与通道都已消失。栈/trap 帧/团队空间的回收是内核私事、对调用方
/// 不可观测，故**不入契约**（那也是延迟回收存在的理由）。
pub fn join(tid: usize, dur: Duration) -> Result<Handoff<bool>, GateError> {
    if target_dead(tid) {
        return if crate::work::unit::task::allocated(tid) {
            Ok(Handoff::Resume(true))
        } else {
            Err(GateError::Denied)
        };
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match block(WakeKey::Task { id: tid }, dur) {
        Handoff::Switch(pa) => Handoff::Switch(pa),
        // 信标已置：目标在「判死 → 入队」的窗口内被回收 ⇒ 当场结论（已回收）。
        Handoff::Resume(()) => Handoff::Resume(true),
    })
}

/// 键退役：放行该键上的**全部**等待者，并留下信标（墓碑）。
///
/// 「目标已回收」与「资源已封印/销毁」是同一件事的两副面孔：这个键再也不会有人
/// 投信，此后到达的等待者必须**当场**得到结论，而不是永远等下去——信标承担这一点
/// （`wake` 只在无人在等时置位；这里无条件置位）。一个信标够用：调用方在入队前都
/// 先探过条件（`join` 探 `target_dead`、`hole::wait` 探就绪位），窗口最多一人。
///
/// 锁纪律同 [`wake`]：只在站点表（L3）内摘除，锁外 transform + 入队。返回唤醒数。
pub(crate) fn wipe(key: WakeKey) -> usize {
    let waiters = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(Site::new);
        site.pend = true;
        core::mem::take(&mut site.waiters)
    };
    for w in &waiters {
        void(w.ticket);
    }
    rise(waiters.into_iter().map(|w| w.task))
}
// ── 操作：唤醒 ──
// ── 操作：唤醒 ──

/// 叫醒一个：摘队首 → 放回就绪。无人在等 → 置信标（防漏唤醒）。返回是否唤到人。
///
/// 消费方 = utask/envcall 与 mail 的投信方；跨核经 steal 再平衡（同 [`redeem`]）。
///
/// **信标可能陈旧**：「信号」与「数据」是两份状态——等待者后来直接取走数据
/// （裸 pull 成功，不经 `wait`）时信标不被消费，下一次 `wait` 就立刻返回「已唤醒」
/// 而实际无数据。故 `wait` 的返回**只是提示**，调用方必须自己复核条件
/// （`hole::wait` 已复核就绪位；有界等待方还须按 deadline 循环，见
/// `docs/dispatch.md` §11.4）。
pub fn wake(key: WakeKey) -> bool {
    let popped = {
        let mut sites = sites(key).lock();
        let site = sites.entry(key).or_insert_with(Site::new);
        let popped = match site.waiters.pop_front() {
            Some(w) => Some(w),
            None => {
                site.pend = true;
                None
            }
        };
        prune(&mut sites, key);
        popped
    };
    let Some(w) = popped else { return false };
    void(w.ticket);
    rise(core::iter::once(w.task));
    true
}

/// 到期兑现：`chrono` 交回的不透明句柄，在这里还原成「谁」。
///
/// 一段走完，不再认识 park / wait / join 的区别：
///   票根 → 升出持票人 → 读它自己那张票上的键 → 从该键的队列里按票号摘出。
/// 陈旧的登记在每一步都自然落空（票根已被 `void`、任务已不阻塞、票号对不上），
/// 故不需要任何「取消」记账。
///
/// 按 tock 堆取到期者（与入队顺序无关）；堆锁先放后取，绝不持堆锁取调度锁
/// （防 ABBA）。返回：本次是否撤出过任务（空闲核的哑睡壳判定用）。
/// 由 trap 路径（S-timer 处理）与空闲核归队时在本 hart 触发。
pub fn redeem() -> bool {
    let due = timer::drain(clock::now());
    // 批量收集再统一 `rise`：一次 kick 收尾（逐条踢会让 IPI 量回到 O(N)）。
    let mut tasks: Vec<Arc<Task>> = Vec::new();
    for handle in due {
        // 票号即到点登记的身份：作废票根并取回持票人（已回收 → 落空）。
        let Some(task) = void(Ticket(handle)) else {
            continue;
        };
        let TaskState::Blocked { key, ticket } = task.state() else {
            continue;
        };
        // 从该键的队列里摘出**这一票**的等待者（票号对不上 = 陈旧，落空）。
        let popped = {
            let mut sites = sites(key).lock();
            let w = sites.get_mut(&key).and_then(|site| {
                site.waiters
                    .iter()
                    .position(|w| w.ticket == ticket)
                    .map(|i| site.waiters.remove(i).expect("idx from position"))
            });
            prune(&mut sites, key);
            w
        };
        let Some(w) = popped else { continue };
        tasks.push(w.task);
    }
    rise(tasks) > 0
}
// ── 操作：回收 ──

/// 回收全部躯壳任务：簿记清理 + 栈 slot/trap 帧归还 + drop。安全：躯壳不在任何核
/// 运行（running/starved 均无引用）。锁纪律：只持 reaped 锁出队，放锁后再取
/// Team.tasks / Space.inner（顺序获取、不嵌套）。
///
/// **入队的任务已经收尾**（退出钩子见 [`reap`]），本函数只做回收——「等收尾」与
/// 「等回收」因此分开：前者是 `Join` 的语义，后者对调用方不可观测。
pub fn bury() {
    loop {
        // 显式作用域取 z：if-let 的临时 guard 会存活到整个循环体（Rust 语义），
        // 导致 husks(L3) 锁跨 Team.tasks 等 L3 表——同层嵌套 lockdep 违规。
        // 块结束即释放 husks 锁。
        let z = {
            let mut husks = HUSKS.lock();
            let Some(z) = husks.pop_front() else {
                break;
            };
            z
        };
        trace::note(EventKind::Room(RoomEvent::Reap { tid: z.ident.id }));
        // 目标已收尾 → 叫醒它的全部 join 等待者（内核驱动，覆盖 fault 死亡）。
        wipe(WakeKey::Task { id: z.ident.id });
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        z.ident.team.prune_tasks(&z);
        // 锁外回收（Team.tasks 已放 → Space.inner=2 合法）：栈 slot + trap 帧
        // 一次 with_flush 经 `Space::release(Span)` 收回——段归还 + PTE 清理 +
        // 刷 TLB；帧随 map drop 归还 frame 池。Span 是 claim 时存进 TaskIdent 的
        // 区间身份（类型同一，不 re-find）。
        z.ident
            .team
            .space
            .release(z.ident.stack)
            .expect("release: span mismatch");
        z.ident
            .team
            .space
            .release(z.ident.frame)
            .expect("release: span mismatch");
        drop(z);
        // 回收完成（栈/帧/团队空间已归还）才计数：done() 成立 ⇔ 全部回收完毕，
        // halt 的关机断言无滞留可验。
        conductor::exit();
    }
}

// ── 退出钩子注册面 ──
//
// mail（dock / ring）在 `boot::init` 把自己的任务退出函数挂到这里。每条收尾的
// 任务按注册顺序跑一次——本域不命名任何子系统，故不知道挂上来的是谁。
type Hook = fn(usize);

static HOOKS: OnceLock<&'static [Hook]> = OnceLock::new();

/// 挂上退出钩子（一次性；由 `boot::init` 调用）。
pub(crate) fn hook(hooks: &'static [Hook]) {
    let _ = HOOKS.set(hooks);
}

/// 对 `tid` 跑一遍挂上的钩子（未挂 = 无事）。
fn hooked(tid: usize) {
    if let Some(hooks) = HOOKS.get() {
        for h in hooks.iter() {
            h(tid);
        }
    }
}

/// 终末释放：清空 messenger 持有的全部 `Arc<Task>`（sites / husks）与全部票根
/// ——Arc<Task> 归零 → Task::drop → MailHolds::drop → 链。由
/// [`scheduler::core::rip`] 在 halt 路径调用。
///
/// 逐表 `take` 出内容、**锁外 drop**：drop 链会取 Space 锁（L2），锁内 drop 即
/// L3→L2 嵌套。站点表分片版循环逐片取——不持跨片锁。
pub(crate) fn rip() {
    for shard in 0..SITE_SHARDS {
        let sites_out = core::mem::take(&mut *shard_at(shard).lock());
        drop(sites_out);
    }
    let husks_out = core::mem::take(&mut *HUSKS.lock());
    drop(husks_out);
    holders().lock().clear(); // 只存 Weak，无 drop 链
    doomed().lock().clear(); // 只存 id，无 Arc
}
// ── 操作：扑杀（suspend / reap / cull / doom）──
//
// 血缘级联的「杀」侧，**两阶段**：先停摆（摘出全部调度/等待容器），再收尾
// （`reap`：钩子 → Reaped → 入躯壳队列）。`doom` 是 hook 里的触发面
// （读父 task 的 heir → 整棵子树两阶段扑杀）。
//
// 两阶段是**正确性要求**，不是优化：钩子会摘门闩，摘门闩会唤醒等待者；若受害者
// 尚未停摆，它可能被别的核偷走并运行，在「已注定要死」的状态下观察到一个已死的
// 资源。他核 Running 任务无法被本核同步拉走（会破坏「Reaped 不在 running 槽」
// 不变量），故走 `doomed` 待杀集合 + SSIP 单点，目标核 trap 自查自退——最终一致。

/// 停摆单线程：摘出全部调度/等待容器 → 置 `Doomed`。返 `true` = 本次停摆了它，
/// 调用方须随后 [`reap`]；`false` = 没动它（已 `Doomed`/`Reaped`、不在任何容器，
/// 或 `Running`——后者已记 doomed + SSIP，待其自退时自己 `reap`）。
///
/// 锁纪律：只持 L3 表，逐表取、放锁后再取下一表（L3 同层绝不嵌套）；锁内不 drop
/// Arc（drop 链触 L2）。
fn suspend(task: &Arc<Task>) -> bool {
    let taken = match task.state() {
        TaskState::Reaped | TaskState::Doomed => false,
        TaskState::Held => {
            // 未放行的引导线程：从 Team.held 摘出（不是它则放回）。
            let team = task.ident.team.clone();
            match team.take_held() {
                Some(held) if Arc::ptr_eq(&held, task) => true,
                Some(held) => {
                    team.hold(&held);
                    false
                }
                None => false,
            }
        }
        TaskState::Starved => crate::work::room::scheduler::core::remove_from_starved(task),
        TaskState::Blocked { key, ticket } => {
            // 读票直达：键指出容器、票号指出队列里的哪一个——不必扫分片。
            let popped = {
                let mut sites = sites(key).lock();
                let w = sites.get_mut(&key).and_then(|site| {
                    site.waiters
                        .iter()
                        .position(|w| w.ticket == ticket)
                        .map(|i| site.waiters.remove(i).expect("idx from position"))
                });
                prune(&mut sites, key);
                w
            };
            if popped.is_none() {
                return false;
            }
            // 作废票根 + 消音到点（[`void`] 幂等）。
            void(ticket);
            true
        }
        TaskState::Running { .. } => {
            if let Some(hart) = crate::work::room::scheduler::core::running_hart(task) {
                doomed().lock().insert(task.ident.id);
                crate::work::room::conductor::nudge(hart);
            }
            false
        }
    };
    if taken {
        let mut t = task.clone();
        Task::exclusive(&mut t).transform(TaskState::Doomed);
    }
    taken
}

/// 扑杀整棵血缘子树（**两阶段**）：
///
///   1. 收集：显式工作栈沿 heir 收齐全部任务（防爆栈）；
///   2. 停摆：逐个 [`suspend`]——`Running` 分支只记 doomed + SSIP，它自退时自己 `reap`；
///   3. 收尾：逐个 [`reap`]（钩子 → Reaped → 入躯壳队列）。
///
/// **阶段边界即安全边界**：第 3 阶段的钩子会摘门闩、唤醒等待者，而此刻全部受害者
/// 都已停摆，没有「被唤醒后还能跑」的中间态。栈/名单都是局部 Vec（锁外分配），
/// 全程不持任何锁。
pub(crate) fn cull(roots: &[Arc<Team>]) {
    let mut work: Vec<Arc<Team>> = roots.to_vec();
    let mut tasks: Vec<Arc<Task>> = Vec::new();
    while let Some(t) = work.pop() {
        // 先取本域成员快照（放锁），再收任务与子域；不持 team.tasks 锁。
        for weak_task in t.tasks_snapshot() {
            if let Some(task) = weak_task.upgrade() {
                work.extend(task.heirs());
                tasks.push(task);
            }
        }
    }
    let victims: Vec<Arc<Task>> = tasks.into_iter().filter(|t| suspend(t)).collect();
    for task in victims {
        reap(task);
    }
}

/// 级联触发（挂 exit_hook）：读父 task 的 heir → 两阶段扑杀整棵血缘子树。
pub(crate) fn doom(tid: usize) {
    if let Some(task) = lookup_task_by_id(tid) {
        cull(&task.heirs());
    }
}

/// trap(SupervisorSoft) 自退查询：本 hart 当前 running 任务是否被判死。
/// 在则摘出待杀标记并返回 true（调用方 quit）；否则 false。
pub(crate) fn take_doomed(tid: usize) -> bool {
    doomed().lock().remove(&tid)
}

// ── 内部辅助 ──

impl Site {
    fn new() -> Self {
        Self {
            pend: false,
            waiters: VecDeque::new(),
        }
    }
}
