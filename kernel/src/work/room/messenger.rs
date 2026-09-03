// 事件队列（messenger）— 任务不在 running 槽时的状态机。
//
// 任务离开 running 槽有三种过渡：park（按 deadline 挂起）、wait（按键等信号）、
// reap（退出）。三种共用 [`scheduler::core::Scheduler::disown_and_install_next`]
// 跨边界原语——先借 scheduler 把 running 卸下（槽位 settled：装下一 starved 或
// 降级 Last），再挂到本域的簿记/计时器上。恢复路径分两类：wake_by_event（信号到）
// 和 drain_expired（timer 到期），都把任务转 Starved 推回 scheduler 本核 + yell。
//
// 簿记：parked（deadline 句柄 → task）、sites（key → pend+waiters）、times
// （tock 句柄 → key）、reaped（Arc<Task> 队列）。四张表全 L3，3→3 嵌套禁止。
// 锁序：park / wait / reap 路径 1 → 3 嵌套合法（持调度锁时入簿）；drain_expired
// 先放堆锁再取 sites/times/parked，再 push 到 scheduler（push 仅持调度锁），
// 绝不持任 L3 取调度锁（防 ABBA）。
//
// 反向耦合清零：dock / ring 的 task_exit 反向耦合走两步拆——step 5 引入 exit
// hook 注册面后，clear_loop 不再硬编码子系统名。本 step 暂留直调作为过渡。

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::work::room::conductor;
use crate::work::room::scheduler::core::current;
use crate::work::unit::{
    task::{BlockReason, Task, TaskState},
};

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
        WaitKey(((asid & 0xFFFF) << 48) | (va & ((1usize << 48) - 1)))
    }

    /// 直接以本体值构造事件键（dock 键路径：`DOCK_KEY_TAG | id` 全局唯一，不经
    /// compose——调用方（envcall 边界）已按标记位分流）。
    pub fn from_raw(raw: usize) -> WaitKey {
        WaitKey(raw)
    }
}

/// 一个事件键的等待位：遗留信号（闩）+ 等待者队列。
struct WaitSite {
    /// 遗留信号（闩）：wake 无等待者 → 置位；wait 见位 → 消费即回（防漏唤醒）。
    pend: bool,
    /// 等待者（FIFO）；每项携带超时句柄（无超时 = None）。
    waiters: VecDeque<Waiter>,
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

/// 事件等待表（Level::L3，与 parked 同级；绝不 3→3 嵌套）。
///
/// 单一注册表 = 单一线性化点：wait / wake 与超时到期都经本表锁摘除，败者见空即弃
/// （杜绝双唤醒）。键不主动销毁（pend 留滞为闩，语义见设计）。
fn wait_sites() -> &'static SpinLock<HashMap<WaitKey, WaitSite>> {
    static SITES: OnceLock<SpinLock<HashMap<WaitKey, WaitSite>>> = OnceLock::new();
    SITES.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 超时旁路：tock 句柄 → 事件键（timer 到期分派用）。任务本体只在 wait_sites
/// 注册；本表只放键（不含 Arc）——两条唤醒路仍都经 wait_sites 锁摘除。
fn wait_times() -> &'static SpinLock<HashMap<u64, WaitKey>> {
    static TIMES: OnceLock<SpinLock<HashMap<u64, WaitKey>>> = OnceLock::new();
    TIMES.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 全局 reaped 队列（Level::L3，与 Team.tasks 同级）：延迟回收——不能在
/// 自己正在用的栈上回收自己；clear_loop  统一回收。
pub(super) static REAPED: SpinLock<VecDeque<Arc<Task>>> =
    SpinLock::new_level(Level::L3, VecDeque::new());

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
/// 变更）。`dur == Duration::MAX` → 永久（无 tock）；否则登记超时。
/// 返回下一帧 PA（若 scheduler 装了下一 starved）。
pub fn wait(key: WaitKey, dur: Duration) -> Option<usize> {
    let cond = current();

    // pend 消费路径：信号已至 → 任务不阻塞（续跑）；锁 sites 短暂。
    {
        let mut sites = wait_sites().lock();
        let site = sites.entry(key).or_insert_with(WaitSite::new);
        if site.pend {
            site.pend = false;
            let pa = cond
                .running_frame_pa()
                .expect("wait with no running task");
            return Some(pa);
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
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        reason: BlockReason::Wait { wake_at },
    });
    // 入 sites[].waiters]队尾
    wait_sites()
        .lock()
        .get_mut(&key)
        .expect("site just observed")
        .waiters
        .push_back(Waiter { task, tock });
    // 超时登记：先旁路簿记、后 tock（堆可见 ⇒ 簿记必在，同 park 纪律）
    if let (Some(wake_at), Some(handle)) = (wake_at, tock) {
        wait_times().lock().insert(handle, key);
        timer::tock(handle, wake_at);
    }

    next_pa
}

/// mark_reaped：Running → Reaped 入全局 reaped 队列（延迟回收——不能在
/// 自己正在用的栈上回收自己；计数在 clear_loop 完成后递增）。
pub fn mark_reaped() -> Option<usize> {
    let cond = current();
    let (mut exited, next_pa) = cond.disown_and_install_next();
    debug_assert!(
        matches!(exited.state(), TaskState::Running { .. }),
        "running 容器里不是 Running 任务"
    );
    trace::note(EventKind::Room(RoomEvent::Exit {
        tid: exited.ident.id,
    }));
    Task::exclusive(&mut exited).transform(TaskState::Reaped);
    // 离核且无后继装槽 → 槽已 settled（disown_and_install_next 内 demote 或
    // 装下一）；团队 Arc 归零即回收——地址空间随释放。
    REAPED.lock().push_back(exited); // L3 单独锁，1 → 3 顺序、不嵌套
    // 注意：回收计数（conductor::exit）不在入队时递增——须等 clear_loop 完成栈/
    // trap 帧/团队空间归还后再计数，否则最后任务退出时另一核见 REAPED==PUSHED
    // 立即 halt，本核 clear_loop 未及回收 → 关机断言误报帧泄漏。
    next_pa
}

// ── 操作：唤醒 ──

/// wake_by_event：waiters 非空 → 唤醒队首（Blocked → Starved 推送本核 + yell）；
/// 空 → pend 置位（防漏唤醒）。返回是否唤到人。消费方 = utask/envcall；
/// 跨核唤醒经 steal 再平衡（与 drain_expired 一致）。
pub fn wake(key: WaitKey) -> bool {
    let popped = {
        let mut sites = wait_sites().lock();
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
    // 摘超时旁路：堆项留至到期被 drain 空闲丢弃（不 untock——已 drain 的句柄再
    // untock 会永久污染 cancelled 表，见 drain 语义）
    if let Some(handle) = w.tock {
        wait_times().lock().remove(&handle);
    }
    let mut task = w.task;
    Task::exclusive(&mut task).transform(TaskState::Starved);
    trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
    current().push(task);
    conductor::yell();
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
        // 事件等待超时：旁路表命中 → 从 wait-site 摘（by tock == handle）唤醒
        let wait_key = wait_times().lock().remove(&handle);
        if let Some(key) = wait_key {
            let popped = {
                let mut ws = wait_sites().lock();
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
            if let Some(w) = popped {
                woke = true;
                let mut task = w.task;
                Task::exclusive(&mut task).transform(TaskState::Starved);
                trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
                current().push(task);
                conductor::yell();
            }
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
        conductor::yell();
    }
    woke
}

// ── 操作：回收 ──

/// 回收全部 Reaped 任务：簿记清理 + 栈 slot/trap 帧归还 + drop。安全：Reaped
/// 任务不在任何核运行（running/starved 均无引用）。锁纪律：只持 reaped 锁
/// 出队，放锁后再取 Team.tasks / Space.inner（顺序获取、不嵌套）。
///
/// 退出钩子（每条 reaped 任务调一次）经 [`register_exit_hooks`] 注册——mail 在
/// init 时挂 dock::task_exit + ring::task_exit，clear_loop 不直接命名子系统。
pub fn clear_loop() {
    let hooks = exit_hooks();
    loop {
        // 显式作用域取 z：if-let 的临时 guard 会存活到整个循环体（Rust 语义），
        // 导致 reaped(L3) 锁跨 dock/ring 的 task_exit（内部取 task_docks/task_rings
        // 同为 L3）——同层嵌套 lockdep 违规。块结束即释放 reaped 锁。
        let z = {
            let mut reaped = REAPED.lock();
            let Some(z) = reaped.pop_front() else {
                break;
            };
            z
        };
        trace::note(EventKind::Room(RoomEvent::Reap { tid: z.ident.id }));
        // 退出钩子（mail 等注册的）：任务名下全部通道引用逐条递减（dock pier →
        // Hang 判据、quay → Dead 判据；ring → Dead）。需在簿记清理前执行（锁外，
        // 只经通道注册表 L3 —— spawn 路径 1→3 合法，clear_loop 同）。
        for hook in hooks {
            hook(z.ident.id);
        }
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        z.ident.team.prune_tasks(&z);
        // 锁外回收（Team.tasks 已放 → Space.inner=2 合法）：栈 slot + trap 帧
        // 一次 with_flush 经 `Space::release(Span)` 收回——段归还 + PTE 清理 +
        // 刷 TLB；帧随 map drop 归还 frame 池。Span 是 claim 时存进 TaskIdent 的
        // 区间身份（类型同一，不 re-find）。
        z.ident.team.space.release(z.ident.stack).expect("release: span mismatch");
        z.ident.team.space.release(z.ident.frame).expect("release: span mismatch");
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
/// reaped 四张表）——Arc<Task> 归零 → Task::drop → MailHolds::drop → 链。
/// 由 [`scheduler::core::rip`] 在 halt 路径调用；mail 接入点由 task.mail
/// 析构透传释放（无需 mail 自有关闭钩子）。
pub(crate) fn rip() {
    parked().lock().clear();
    wait_sites().lock().clear();
    wait_times().lock().clear();
    REAPED.lock().clear();
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