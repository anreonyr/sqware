// 事件队列（messenger）— 任务不在 running 槽时的状态机。
//
// 任务离开 running 槽有三种过渡：park（按 deadline 挂起）、wait（按键等信号）、
// reap（退出）。三种共用 [`scheduler::core::Scheduler::disown_and_install_next`]
// 跨边界原语——先借 scheduler 把 running 卸下（槽位 settled：装下一 starved 或
// 降级 Last），再挂到本域的簿记/计时器上。恢复路径分两类：wake_by_event（信号到）
// 和 drain_expired（timer 到期），都把任务转 Starved 推回 scheduler 本核 + kick。
//
// 簿记：parked（deadline 句柄 → task）、sites（key → pend+waiters）、times
// （tock 句柄 → key）、husks（Arc<Task> 队列）。四张表全 L3，3→3 嵌套禁止。
// 锁序：**L1（调度器）与 L3（本域四表）任何方向都不得嵌套**——持任一 L3 期间
// 不得调用 scheduler 的任何加锁方法，也不得在锁内 drop `Arc<Task>`（drop 链会
// 取 Space 锁 L2）。各路径的写法统一为「作用域内取、作用域外用」：wait/wake/
// drain_expired 在块内摘出 Waiter、块外 push 回 scheduler；clear_loop 块内出队、
// 块外回收；rip 块内 take 整表、块外 drop。
//
// 反向耦合清零：dock / ring 的 task_exit 反向耦合走两步拆——step 5 引入 exit
// hook 注册面后，clear_loop 不再硬编码子系统名。本 step 暂留直调作为过渡。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use hashbrown::{HashMap, HashSet};

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::{current, lookup_task_by_id};
use crate::work::unit::gate::GateError;
use crate::work::unit::task::{BlockReason, Task, TaskState};
use crate::work::unit::team::Team;

// ── 句柄分配 ──

/// park / wait (with timeout) 的句柄分配器——messenger 自管；「先入簿、后 tock」
/// 闭合竞态（堆可见 ⇒ 簿记必在——drain_expired 按句柄摘除绝不会命中空簿记）。
static HANDLE: AtomicUsize = AtomicUsize::new(0);

// ── 类型 ──

/// 事件等待键（newtype）：核心不解释组成，纯匹配。合成走 [`WaitKey::compose`]
/// （适配层在 envcall 边界调用，并入空间身份）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaitKey(usize);

impl WaitKey {
    /// 合成 = asid 高 16 位 || 用户 VA 低 48 位。单射需 va < 2^48（Sv39/48 满足；
    /// Sv57 若启用需重定布局）。与 fence::key 同源意：跨空间同 VA 不得混淆。
    pub fn compose(asid: usize, va: usize) -> WaitKey {
        // 用 #[inline(never)] helper 承担 mask 计算（见 §13.10 A 待办方向 A）：
        // size 优化在闭包多次内联 compose 时，会把 `+1`（来自 hole::key(.., Pull)
        // 的 `| 1` 折叠）算在 mask 上 → mask 错联。helper 强制每次调用独立计算。
        WaitKey(((asid & 0xFFFF) << 48) | low48(va))
    }
}

#[inline(never)]
fn low48(va: usize) -> usize {
    // 0x0000_FFFF_FFFF_FFFF 字面量替代 ((1usize << 48) - 1)：让编译器视作
    // 已折叠常量；helper 整体 inline(never)，杜绝 size 优化把 mask 寄存器
    // 跨调用复用。
    va & 0x0000_FFFF_FFFF_FFFFusize
}

// 内核线程面（`TaskBuilder::closure`）的配套接口：目录已移出内核（跑在 `task-dir`
// 域里），树内暂无使用者——保留备用，故显式关掉 dead_code。
#[allow(dead_code)]
impl WaitKey {
    /// 直接以本体值构造事件键（dock 键路径：`DOCK_KEY_TAG | id` 全局唯一，不经
    /// compose——调用方（envcall 边界）已按标记位分流）。
    pub fn from_raw(raw: usize) -> WaitKey {
        WaitKey(raw)
    }
    pub fn into_raw(self) -> usize {
        self.0
    }
}

/// 一个事件键的等待位：遗留信号（闩）+ 等待者队列。
struct WaitSite {
    /// 遗留信号（闩）：wake 无等待者 → 置位；wait 见位 → 消费即回（防漏唤醒）。
    pend: bool,
    /// 等待者（FIFO）；每项携带超时句柄（无超时 = None）。
    waiters: VecDeque<Waiter>,
}

/// 交接：本次调用之后由谁占住处理器。
///
/// 三态各自独立，不可合并：`Resume` 是**没离核**（当前帧由调用方持有——
/// messenger 不知道也不该知道当前帧是什么），另两态是**已离核**，区别只在本核
/// starved 是否装得上下一位。
pub enum Handoff {
    /// 未离核：继续跑调用方的当前帧。
    Resume,
    /// 已离核：切到该帧（本核 starved 队首已装槽）。
    Switch(usize),
    /// 已离核且本核无后继：由适配层取活（`run()`）。
    Idle,
}

struct Waiter {
    task: Arc<Task>,
    tock: Option<u64>,
}

// ── 簿记表（全部 L3） ──

/// deadline-keyed 等待者（park 路径）：句柄 → 任务。条目即任务本身，阻塞原因
/// （含 wake_at）在任务的 Blocked(Park) 载荷里——映射退化为「句柄 → 唯一 Arc<Task>」。
fn parked() -> &'static SpinLock<HashMap<u64, Arc<Task>>> {
    static PARKED: OnceLock<SpinLock<HashMap<u64, Arc<Task>>>> = OnceLock::new();
    PARKED.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 事件等待表的分片数。每片 = 一把 L3 锁 + 一个 HashMap；wait/wake/drain
/// 按 [`site_shard`] 纯函数路由到同片，跨片互不阻塞——把单点串行竞争降到
/// 1/SITE_SHARDS（典型 16）。分片数取 2 的幂：位与替代 mod。
const SITE_SHARDS: usize = 16;
const SITE_SHARDS_MASK: usize = SITE_SHARDS - 1;

/// 事件键 → 分片（pure function，所有路径一致：wait / wake / drain 都经此）。
/// splitmix64 折叠 64→32 后按位与 SHARDS 掩码——高位低位的熵都被采样。
#[inline]
fn site_shard(key: WaitKey) -> usize {
    let h = (key.0 as u64).wrapping_mul(0x9E3779B97F4A7C15);
    ((h >> 32) ^ h) as usize & SITE_SHARDS_MASK
}

/// 事件等待表（Level::L3，与 parked 同级；绝不 3→3 嵌套）。**分片版**：每片
/// 一把 L3 锁 + HashMap，单一线性化点缩小到一片——wait/wake 跨片并行。
///
/// 锁纪律（与原版同级）：仍是 L3、与 parked 同级、可与 timer 锁共存但**绝不
/// 3→3 嵌套**（rip 路径循环逐片清，禁持跨片锁）。同 key 的所有 waiters 必落
/// 在同一分片（`site_shard` 纯函数保证），wake 不必跨片扫描。
fn wait_sites(shard: usize) -> &'static SpinLock<HashMap<WaitKey, WaitSite>> {
    static SHARDS: OnceLock<Box<[SpinLock<HashMap<WaitKey, WaitSite>>]>> = OnceLock::new();
    let arr: &'static [SpinLock<HashMap<WaitKey, WaitSite>>] = SHARDS.get_or_init(|| {
        let mut v: Vec<SpinLock<HashMap<WaitKey, WaitSite>>> = Vec::with_capacity(SITE_SHARDS);
        for _ in 0..SITE_SHARDS {
            v.push(SpinLock::new_level(Level::L3, HashMap::new()));
        }
        v.into_boxed_slice()
    });
    &arr[shard]
}

/// 超时旁路：tock 句柄 → 事件键（timer 到期分派用）。任务本体只在 wait_sites
/// 注册；本表只放键（不含 Arc）——两条唤醒路仍都经 wait_sites 锁摘除。
fn wait_times() -> &'static SpinLock<HashMap<u64, WaitKey>> {
    static TIMES: OnceLock<SpinLock<HashMap<u64, WaitKey>>> = OnceLock::new();
    TIMES.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 全局躯壳队列（Level::L3，与 Team.tasks 同级）：延迟回收——不能在
/// 自己正在用的栈上回收自己；clear_loop  统一回收。
pub(super) static HUSKS: SpinLock<VecDeque<Arc<Task>>> =
    SpinLock::new_level(Level::L3, VecDeque::new());

/// 待杀集合（doomed）：`kill` 点名他核 Running 任务时记入，目标核 trap 自查
/// 自退。无主簿记——只存 task_id，不持 `Arc<Task>`（防「杀者撑着被杀者」）。
/// Level::L3，与 parked/sites 同级。
fn doomed() -> &'static SpinLock<HashSet<usize>> {
    static T: OnceLock<SpinLock<HashSet<usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashSet::new()))
}

/// Join 等待站点：遗留信号（pend，目标已回收但当时无人等）+ FIFO 等待者。
///
/// `pend` 闭合「判死 → 入簿」窗口：目标在窗口内被回收时 `wake_joiners` 置 pend，
/// 入簿者见到即当场撤销阻塞——**无须在持 joins 锁时再查注册表**（那是 3→3）。
struct JoinSite {
    pend: bool,
    waiters: VecDeque<Waiter>,
}

/// Join 等待表（Level::L3）：目标 tid → 站点。
///
/// 条目持等待者的 `Arc<Task>`，故 `rip` 必须清空（否则关机审计把等待者的空间
/// 算成泄漏）。与 `wait_sites` 同形、同为 L3，绝不 3→3 嵌套。
fn joins() -> &'static SpinLock<HashMap<usize, JoinSite>> {
    static J: OnceLock<SpinLock<HashMap<usize, JoinSite>>> = OnceLock::new();
    J.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// Join 超时旁路（Level::L3）：tock 句柄 → 目标 tid（只存键，无 Arc）。
fn join_times() -> &'static SpinLock<HashMap<u64, usize>> {
    static T: OnceLock<SpinLock<HashMap<u64, usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

// ── 操作：挂起（用 scheduler::core::disown_and_install_next） ──

/// park：Running → Blocked(Park{wake_at})。借 scheduler 取走 running；句柄 +
/// 入 parked + timer::tock；返回下一帧 PA（若 scheduler 装了下一 starved）。
pub fn park(duration: Duration) -> Option<usize> {
    let cond = current();
    let (mut task, next_pa) = cond.disown_and_install_next();

    let wake_at = clock::now().add(duration).as_ticks();
    trace::note(EventKind::Room(RoomEvent::Park {
        tid: task.ident.id,
        wake_at: wake_at as usize,
    }));
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        reason: BlockReason::Park { wake_at },
    });
    // 锁序 HANDLE → parked → timer（防跨锁竞态；1 → 3 顺序、不嵌套）：
    let handle = HANDLE.fetch_add(1, Ordering::Relaxed) as u64;
    // debug: 同一任务不得在 parked 簿记中重复登记（两次 park 同一任务 = 唤醒后
    // 重复入队 → 多容器强持有）。持 parked 锁遍历核对（锁内不做分配）。
    #[cfg(debug_assertions)]
    {
        let p = parked().lock();
        if p.values().any(|t| Arc::ptr_eq(t, &task)) {
            panic!(
                "park: task #{} '{}' already in parked map (double park, handle {handle})",
                task.ident.id, task.ident.name
            );
        }
    }
    parked().lock().insert(handle, task);
    timer::tock(handle, wake_at);

    next_pa
}

/// 事件等待：Running → Blocked(Wait)。pend 存在 → 消费即回（不阻塞，无状态
/// 变更，[`Handoff::Resume`]）。`dur == Duration::MAX` → 永久（无 tock）；否则
/// 登记超时。
pub fn wait(key: WaitKey, dur: Duration) -> Handoff {
    let cond = current();

    // pend 消费路径：信号已至 → 任务不阻塞（续跑）；锁 sites 短暂，锁内不问
    // 调度器（当前帧是调用方的知识，不是本域的）。
    {
        let mut sites = wait_sites(site_shard(key)).lock();
        let site = sites.entry(key).or_insert_with(WaitSite::new);
        if site.pend {
            site.pend = false;
            return Handoff::Resume;
        }
    }

    let (mut task, next_pa) = cond.disown_and_install_next();
    let (wake_at, tock) = if dur == Duration::MAX {
        (None, None)
    } else {
        let wake_at = clock::now().add(dur).as_ticks();
        let handle = HANDLE.fetch_add(1, Ordering::Relaxed) as u64;
        (Some(wake_at), Some(handle))
    };
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id,
        key: key.0,
    }));
    // 入 sites[].waiters]队尾。此处**必须再查 pend**：首段 pend 检查与派单之间
    // 有窗口（disown_and_install_next 取调度锁 L1，持 sites L3 期间不可取），
    // wake 可能在此窗口置 pend——若此刻已注册 waiter 而不消费 pend，该 waiter 永
    // 不被唤醒（pend 只会被"下一次 wait"消费）。闭环：注册入锁后见 pend → 消费、
    // 撤销本次阻塞（任务已在槽外，回收为 Starved 即可——由 run 再接走）。
    let mut sites = wait_sites(site_shard(key)).lock();
    let site = sites.get_mut(&key).expect("site just observed");
    if site.pend {
        // 窗口内 wake 已至——本任务按「已唤醒」处理，不阻塞。
        site.pend = false;
        drop(sites);
        Task::exclusive(&mut task).transform(TaskState::Starved);
        trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
        current().push(task);
        // 已入本核 starved（run 会再接走）；无后备帧则交 run 取活。
        return match next_pa {
            Some(pa) => Handoff::Switch(pa),
            None => Handoff::Idle,
        };
    }
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        reason: BlockReason::Wait { wake_at },
    });
    site.waiters.push_back(Waiter { task, tock });
    drop(sites);
    // 超时登记：先旁路簿记、后 tock（堆可见 ⇒ 簿记必在，同 park 纪律）
    if let (Some(wake_at), Some(handle)) = (wake_at, tock) {
        wait_times().lock().insert(handle, key);
        timer::tock(handle, wake_at);
    }

    match next_pa {
        Some(pa) => Handoff::Switch(pa),
        None => Handoff::Idle,
    }
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
fn die(mut task: Arc<Task>) {
    match task.state() {
        TaskState::Reaped => return,
        TaskState::Doomed => {}
        _ => Task::exclusive(&mut task).transform(TaskState::Doomed),
    }
    for hook in exit_hooks() {
        hook(task.ident.id);
    }
    Task::exclusive(&mut task).transform(TaskState::Reaped);
    HUSKS.lock().push_back(task); // L3 单独锁，1 → 3 顺序、不嵌套
}

/// mark_reaped：离核装槽 → [`die`]（收尾 + 入队）→ 返下一帧 PA。
///
/// 延迟回收的理由是**回收**而非收尾：不能在自己正在用的栈上回收自己，故栈/trap
/// 帧/团队空间留到 `clear_loop`；收尾（钩子）在此刻就做完了。
pub fn mark_reaped() -> Option<usize> {
    let cond = current();
    // 离核且无后继装槽 → 槽已 settled（disown_and_install_next 内 demote 或
    // 装下一）；团队 Arc 归零即回收——地址空间随释放。
    let (exited, next_pa) = cond.disown_and_install_next();
    debug_assert!(
        matches!(exited.state(), TaskState::Running { .. }),
        "running 容器里不是 Running 任务"
    );
    trace::note(EventKind::Room(RoomEvent::Exit {
        tid: exited.ident.id,
    }));
    die(exited);
    // 注意：回收计数（conductor::exit）不在入队时递增——须等 clear_loop 完成栈/
    // trap 帧/团队空间归还后再计数，否则最后任务退出时另一核见 REAPED==PUSHED
    // 立即 halt，本核 clear_loop 未及回收 → 关机断言误报帧泄漏。
    next_pa
}

// ── 操作：等目标回收（Join） ──

/// `Join` 的结论。未挂起的两态（`Dead` / `Alive`）与「已挂起」用类型分开——
/// 适配层据此写 a0（挂起路径读到的 a0 是挂起前预置值，故只能预置 0）。
pub enum Joined {
    /// 未挂起：目标已死**且收尾完成**（退出钩子已跑完——见 [`die`]）。
    Dead,
    /// 未挂起：目标仍在（`millis == 0` 探测）。
    Alive,
    /// 已挂起：切到该帧（None = 本核无后继，适配层 `run()` 取活）。
    Parked(Option<usize>),
}

/// 目标是否已死透。注册表只存 `Weak` 且从不清理：升级失败 ⇒ 已分配过就是
/// 「已回收」；从未分配 ⇒ 非法 id（调用方另判 `Denied`）。
///
/// `Reaped` 由 [`die`] 独占置位（钩子之后），故本判据为真 ⇔ **收尾已完成**。
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
/// 唤醒由**内核驱动**——目标收尾（含 fault isolation 杀）时 [`wake_joiners`]
/// 叫醒全部等待者，故用户态跑不到的死亡也能被观察到。
///
/// 「结束」= 目标已死**且退出钩子（通道级联 + 能力级联）已跑完**——即返回真时，
/// 它名下的门闩与通道都已消失。栈/trap 帧/团队空间的回收是内核私事、对调用方
/// 不可观测，故**不入契约**（那也是延迟回收存在的理由）。
pub fn join(tid: usize, dur: Duration) -> Result<Joined, GateError> {
    if target_dead(tid) {
        return if crate::work::unit::task::allocated(tid) {
            Ok(Joined::Dead)
        } else {
            Err(GateError::Denied)
        };
    }
    if dur == Duration::ZERO {
        return Ok(Joined::Alive);
    }
    let cond = current();
    let (mut task, next_pa) = cond.disown_and_install_next();
    let (wake_at, tock) = if dur == Duration::MAX {
        (None, None)
    } else {
        let wake_at = clock::now().add(dur).as_ticks();
        let handle = HANDLE.fetch_add(1, Ordering::Relaxed) as u64;
        (Some(wake_at), Some(handle))
    };
    trace::note(EventKind::Room(RoomEvent::Wait {
        tid: task.ident.id,
        key: tid,
    }));
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        reason: BlockReason::Join { tid, wake_at },
    });
    // 竞态闭合：disown 期间目标可能已被 reap——`wake_joiners` 会置 pend。入簿
    // 只持 joins 一把锁（**锁内绝不查注册表**：那是 L3→L3 嵌套，lockdep 会拒）。
    let queued = {
        let mut j = joins().lock();
        let site = j.entry(tid).or_insert_with(|| JoinSite {
            pend: false,
            waiters: VecDeque::new(),
        });
        if site.pend {
            site.pend = false;
            false
        } else {
            site.waiters.push_back(Waiter {
                task: task.clone(),
                tock,
            });
            true
        }
    };
    if !queued {
        Task::exclusive(&mut task).transform(TaskState::Starved);
        current().push(task);
        return Ok(Joined::Parked(next_pa));
    }
    drop(task);
    // 超时登记：先旁路簿记、后 tock（堆可见 ⇒ 簿记必在，同 park 纪律）
    if let (Some(wake_at), Some(handle)) = (wake_at, tock) {
        join_times().lock().insert(handle, tid);
        timer::tock(handle, wake_at);
    }
    Ok(Joined::Parked(next_pa))
}

/// 目标回收时叫醒其全部 join 等待者（`clear_loop` 每条 reaped 任务调一次）。
///
/// 锁纪律同 `wake`：只在 joins（L3）内摘除，锁外 transform + 入队；不 mute
/// （句柄留待 drain 空闲丢弃——已 drain 的句柄再 mute 会污染 cancelled 表）。
fn wake_joiners(tid: usize) {
    let waiters = {
        let mut j = joins().lock();
        let site = j.entry(tid).or_insert_with(|| JoinSite {
            pend: false,
            waiters: VecDeque::new(),
        });
        if site.waiters.is_empty() {
            // 无人在等：留信标——之后入簿者见 pend 即当场撤销阻塞。
            site.pend = true;
            return;
        }
        core::mem::take(&mut site.waiters)
    };
    let mut woke = false;
    for w in waiters {
        if let Some(h) = w.tock {
            join_times().lock().remove(&h);
        }
        let mut task = w.task;
        Task::exclusive(&mut task).transform(TaskState::Starved);
        trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
        current().push(task);
        woke = true;
    }
    if woke {
        conductor::kick();
    }
}

// ── 操作：唤醒 ──

/// wake_by_event：waiters 非空 → 唤醒队首（Blocked → Starved 推送本核 + kick）；
/// 空 → pend 置位（防漏唤醒）。返回是否唤到人。消费方 = utask/envcall；
/// 跨核唤醒经 steal 再平衡（与 drain_expired 一致）。
///
/// **pend 可能变陈旧**：站点不回收，且「信号」与「数据」是两份状态——若等待者后来
/// 直接取走了数据（裸 pull 成功，不经 `wait`），pend 不会被消费，下一次 `wait` 就
/// 会立刻返回「已唤醒」而实际无数据。故 `wait` 的返回**只是提示**，调用方必须自己
/// 复核条件（`hole::wait` 已复核就绪位；有界等待方还须按 deadline 循环，见
/// `docs/dispatch.md` §11.4）。
pub fn wake(key: WaitKey) -> bool {
    let popped = {
        let mut sites = wait_sites(site_shard(key)).lock();
        let site = sites.entry(key).or_insert_with(WaitSite::new);
        match site.waiters.pop_front() {
            Some(w) => Some(w),
            None => {
                site.pend = true;
                None
            }
        }
    };
    let Some(w) = popped else {
        return false;
    };
    // 摘超时旁路：堆项留至到期被 drain 空闲丢弃（不 mute——已 drain 的句柄再
    // mute 会永久污染 cancelled 表，见 drain 语义）
    if let Some(handle) = w.tock {
        wait_times().lock().remove(&handle);
    }
    let mut task = w.task;
    Task::exclusive(&mut task).transform(TaskState::Starved);
    trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
    current().push(task);
    conductor::kick();
    true
}

/// drain_expired：从到期句柄按 parked / sites 映射摘除任务，Blocked → Starved
/// 入本核 starved。
///
/// 按 tock 堆取到期者（与入队顺序无关）；队列锁/堆锁先放后取，绝不持队列锁取
/// 调度锁（防 ABBA）。返回：本次是否撤出过任务（wait 的哑睡壳判定用）。
/// 由 trap 路径（S-timer 处理）在本 hart 触发；`pub`（scheduler 之外消费）。
pub fn drain_expired() -> bool {
    let due = timer::drain(clock::now());
    let mut woke = false;
    for handle in due {
        // Join 超时：旁路表命中 → 从 joins[目标 tid] 按 tock 摘出唤醒
        let join_tid = join_times().lock().remove(&handle);
        if let Some(tid) = join_tid {
            let popped = {
                let mut j = joins().lock();
                j.get_mut(&tid).and_then(|site| {
                    site.waiters
                        .iter()
                        .position(|w| w.tock == Some(handle))
                        .map(|i| site.waiters.remove(i).expect("idx from position"))
                })
            };
            let Some(w) = popped else { continue };
            woke = true;
            let mut task = w.task;
            Task::exclusive(&mut task).transform(TaskState::Starved);
            trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
            current().push(task);
            continue;
        }
        // 事件等待超时：旁路表命中 → 从 wait-site 摘（by tock == handle）唤醒
        let wait_key = wait_times().lock().remove(&handle);
        if let Some(key) = wait_key {
            let popped = {
                let mut ws = wait_sites(site_shard(key)).lock();
                if let Some(site) = ws.get_mut(&key) {
                    if let Some(idx) = site.waiters.iter().position(|w| w.tock == Some(handle)) {
                        Some(site.waiters.remove(idx).expect("idx from position"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            // 空 popped 不吼：句柄已被他路唤醒，事件已消化（woke 不置位）。
            let Some(w) = popped else { continue };
            woke = true;
            let mut task = w.task;
            Task::exclusive(&mut task).transform(TaskState::Starved);
            trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
            current().push(task);
            continue;
        }
        // park 到期（原路径）
        let Some(mut task) = parked().lock().remove(&handle) else {
            // 已取消/已由他路唤醒：跳过（堆项随 drain 已丢弃）
            continue;
        };

        woke = true;
        Task::exclusive(&mut task).transform(TaskState::Starved);
        trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
        current().push(task);
    }
    // 批量踢：循环外一次 SBI IPI（替代原每条吼）。
    // 任务已全部入本核 starved（push 先于踢 = 唤醒方进入 steal 必可见），
    // 单次 kick 把当前最低 set bit 的等待 hart 拉起即可——单 tick IPI 量
    // 从 O(N) → O(1)。`woke` 与"是否真唤醒过"等价：false = 全是空
    // popped/已被取消，无需踢。wake 路径（messenger::wake）单次入单踢。
    if woke {
        conductor::kick();
    }
    woke
}

// ── 操作：回收 ──

/// 回收全部躯壳任务：簿记清理 + 栈 slot/trap 帧归还 + drop。安全：躯壳不在任何核
/// 运行（running/starved 均无引用）。锁纪律：只持 reaped 锁出队，放锁后再取
/// Team.tasks / Space.inner（顺序获取、不嵌套）。
///
/// **入队的任务已经收尾**（退出钩子见 [`die`]），本函数只做回收——「等收尾」与
/// 「等回收」因此分开：前者是 `Join` 的语义，后者对调用方不可观测。
pub fn clear_loop() {
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
        wake_joiners(z.ident.id);
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
// mail（dock / ring）在 `boot::init` 把自己的 task_exit 函数挂到这里。clear_loop
// 每条 reaped 任务按注册顺序调一次——messenger 不直接命名任何子系统。
type ExitHook = fn(usize);

static EXIT_HOOKS: OnceLock<&'static [ExitHook]> = OnceLock::new();

/// 注册任务退出钩子（一次性；由 `boot::init` 调用）。
pub(crate) fn register_exit_hooks(hooks: &'static [ExitHook]) {
    let _ = EXIT_HOOKS.set(hooks);
}

/// 取当前注册（未注册则空切片——clear_loop 仍可跑，no-op）。
fn exit_hooks() -> &'static [ExitHook] {
    static EMPTY: &[ExitHook] = &[];
    EXIT_HOOKS.get().copied().unwrap_or(EMPTY)
}

/// 终末释放：清空 messenger 持有的全部 Arc<Task>（parked / sites / times /
/// husks 四张表）——Arc<Task> 归零 → Task::drop → MailHolds::drop → 链。
/// 由 [`scheduler::core::rip`] 在 halt 路径调用；mail 接入点由 task.mail
/// 析构透传释放（无需 mail 自有关闭钩子）。
///
/// 逐表 `take` 出内容、**锁外 drop**：drop 链会取 Space 锁（L2），锁内 drop 即
/// L3→L2 嵌套。wait_sites 分片版循环逐片取——不持跨片锁。
pub(crate) fn rip() {
    let parked_out = core::mem::take(&mut *parked().lock());
    drop(parked_out);
    for shard in 0..SITE_SHARDS {
        let sites_out = core::mem::take(&mut *wait_sites(shard).lock());
        drop(sites_out);
    }
    wait_times().lock().clear(); // 只存键，无 Arc，无 drop 链
    let joins_out = core::mem::take(&mut *joins().lock());
    drop(joins_out);
    join_times().lock().clear(); // 只存键，无 Arc
    let husks_out = core::mem::take(&mut *HUSKS.lock());
    drop(husks_out);
    doomed().lock().clear(); // 只存 id，无 Arc
}

// ── 操作：扑杀（suspend / die / cull / doom）──
//
// 血缘级联的「杀」侧，**两阶段**：先停摆（摘出全部调度/等待容器），再收尾
// （`die`：钩子 → Reaped → 入躯壳队列）。`doom` 是 exit_hook 里的触发面
// （读父 task 的 heir → 整棵子树两阶段扑杀）。
//
// 两阶段是**正确性要求**，不是优化：钩子会摘门闩，摘门闩会唤醒等待者；若受害者
// 尚未停摆，它可能被别的核偷走并运行，在「已注定要死」的状态下观察到一个已死的
// 资源。他核 Running 任务无法被本核同步拉走（会破坏「Reaped 不在 running 槽」
// 不变量），故走 `doomed` 待杀集合 + SSIP 单点，目标核 trap 自查自退——最终一致。

/// 停摆单线程：摘出全部调度/等待容器 → 置 `Doomed`。返 `true` = 本次停摆了它，
/// 调用方须随后 [`die`]；`false` = 没动它（已 `Doomed`/`Reaped`、不在任何容器，
/// 或 `Running`——后者已记 doomed + SSIP，待其自退时自己 `die`）。
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
        TaskState::Blocked { reason } => match reason {
            BlockReason::Park { .. } => {
                // parked 按 handle 键存；按 ptr_eq 扫出句柄后摘。
                let handle = {
                    let p = parked().lock();
                    p.iter()
                        .find(|(_, t)| Arc::ptr_eq(t, task))
                        .map(|(h, _)| *h)
                };
                let Some(h) = handle else { return false };
                let removed = parked().lock().remove(&h);
                timer::mute(h);
                removed.is_some()
            }
            BlockReason::Wait { .. } => {
                // wait_sites 分片存 Waiter{task, tock}；按 ptr_eq 扫出 tock 后摘。
                let mut tock: Option<Option<u64>> = None;
                for shard in 0..SITE_SHARDS {
                    let mut ws = wait_sites(shard).lock();
                    for site in ws.values_mut() {
                        if let Some(idx) =
                            site.waiters.iter().position(|w| Arc::ptr_eq(&w.task, task))
                        {
                            let w = site.waiters.remove(idx).expect("idx from position");
                            tock = Some(w.tock);
                            break;
                        }
                    }
                    if tock.is_some() {
                        break;
                    }
                }
                let Some(tock) = tock else { return false };
                // 摘 times 旁路 + mute（先摘簿记、后 mute，同 park 纪律）。
                if let Some(h) = tock {
                    wait_times().lock().remove(&h);
                    timer::mute(h);
                }
                true
            }
            BlockReason::Join { tid, .. } => {
                // joins 按目标 tid 存；按 ptr_eq 扫出本任务后摘除。
                let mut tock: Option<Option<u64>> = None;
                {
                    let mut j = joins().lock();
                    if let Some(site) = j.get_mut(&tid)
                        && let Some(idx) =
                            site.waiters.iter().position(|w| Arc::ptr_eq(&w.task, task))
                    {
                        let w = site.waiters.remove(idx).expect("idx from position");
                        tock = Some(w.tock);
                    }
                }
                let Some(tock) = tock else { return false };
                if let Some(h) = tock {
                    join_times().lock().remove(&h);
                    timer::mute(h);
                }
                true
            }
        },
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
///   2. 停摆：逐个 [`suspend`]——`Running` 分支只记 doomed + SSIP，它自退时自己 `die`；
///   3. 收尾：逐个 [`die`]（钩子 → Reaped → 入躯壳队列）。
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
        die(task);
    }
}

/// 级联触发（挂 exit_hook）：读父 task 的 heir → 两阶段扑杀整棵血缘子树。
pub(crate) fn doom(tid: usize) {
    if let Some(task) = lookup_task_by_id(tid) {
        cull(&task.heirs());
    }
}

/// trap(SupervisorSoft) 自退查询：本 hart 当前 running 任务是否被判死。
/// 在则摘出待杀标记并返回 true（调用方 mark_reaped）；否则 false。
pub(crate) fn take_doomed(tid: usize) -> bool {
    doomed().lock().remove(&tid)
}

// ── 内部辅助 ──

impl WaitSite {
    fn new() -> Self {
        Self {
            pend: false,
            waiters: VecDeque::new(),
        }
    }
}
