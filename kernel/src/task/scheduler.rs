// 任务调度器 — 多核（hart B1）：per-hart 调度锁 + 非阻塞 steal + 动态 trap 栈
//
// 本模块只留「调度」本身：线程模型类型（Task/Team/TaskState/TrapFrame）在
// task.rs，进程/线程创建（spawn/spawn_thread）在 create.rs。
//
// 结构（正交语义区）：
//   Scheduler 方法 — 每核状态上的操作，按正交关切分组 impl：
//       构造（new）/ 就绪队列（enqueue、pop_ready）/ 运行与切换
//       （prepare_resume、install_current、tick、exit_current）/ 查询
//       （current_task_id、with_current_space）
//   自由函数      — 跨核/系统级编排：每核表（init_schedulers）、任务生命周期
//       （SPAWNED/EXITED/HALTING + all_exited/halt_all）、对外 API 入口
//       （tick/exit_current/enqueue 等薄包装：取本核锁 + 委托方法）、跨核偷取
//       （steal + install_stolen）、核入口（enter_first_task/idle_loop）
//   任务阻塞      — Blocked 态 + 睡眠队列（SLEEP_LIST）：sleep（Running→Blocked
//                   入队，wake_at 单调 → 队首即最早到期）与 wake_due（tick 扫描
//                   到期者 Blocked→Ready 出队入调度队列）——任务级等待的通用
//                   模式（未来信号量等原语同持锁纪律）
//
// 多核调度（锁模型 B）：
//   schedulers()[hart] — 每 hart 一把 SpinLock（level 1）：保护该 hart 的
//                      current（运行中，**不在队列**）+ ready（VecDeque）。
//                      **按实际核数动态分配**（boot 时 init_schedulers）。
//   steal            — 跨 hart 取活：先读 ready_lens()[v]（锁外原子读，S 态共享
//                      不失效缓存行）再 try_lock（非阻塞）——空队列不做 RMW，
//                      避免对受害者锁行乒乓；拿不到即跳过（无锁序规则）。
//   idle             — 本 hart 无任务：自核队首 → steal → WFI 休眠（IPI / 残留
//                      定时器唤醒，SLEEPING 位图 + wake_sleepers）；全退出 → halt_all。
//
// 切换语义：tick/exit_current/steal 返回下一任务帧 PA；trap_handler 原样返回，
// trampoline 尾部 j __restore 用 a0 = PA 恢复。**kernel_sp 每切换写入**：把帧
// 交给 __restore 前写 frame.kernel_sp = trap_stack_top(hart_id())（__alltraps
// 从帧读 kernel_sp 上 per-hart trap 栈——steal 迁移后任务在偷取核上运行，写漏
// 会让它跑到旧核的栈上；debug assert 在 trap 入口兜底）。
//
// 锁层级（lock/mod.rs）：schedulers()[hart] = level 1（类型级，彼此不嵌套；
// try_lock steal 免于锁序规则）；Team.tasks = level 3——与 Space.inner = 2
// **禁止嵌套持有**：入队收尾/prune_tasks 是纯 Vec 操作，锁内绝不调 space 方法。
//
// 全退出：SPAWNED/EXITED 原子计数；EXITED == SPAWNED → halt_all（srst + wfi，
// AtomicBool 防双核同时触发）。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::ecall::fid;
use crate::ecall::scall::SArgs;
use crate::lock::{OnceLock, SpinLock};
use crate::machine;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::space::Space;
use crate::runtime::context::TrapContext;
use crate::runtime::trap::{TIMER_INTERVAL, arm_timer};
use crate::runtime::trampoline::{restore, trap_stack_top};
use crate::{ecall, putln};

use super::task::{Task, TaskState};

/// 每 hart 调度器状态：current（运行中，不在队列）+ ready（就绪队列）。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub struct Scheduler {
    /// 所属 hart（决定 trap 栈顶与 ready_lens 计数索引）。
    hart: usize,
    /// 当前运行任务（不在 ready 队列；steal 只从 ready 摘——current 不可被偷）。
    current: Option<Arc<Task>>,
    /// 就绪队列。
    ready: VecDeque<Arc<Task>>,
}

// ── 每核调度器：构造 ──────────────────────────────────────────

impl Scheduler {
    /// 构造（init_schedulers 按实际核数逐 hart 建）。
    const fn new(hart: usize) -> Scheduler {
        Scheduler {
            hart,
            current: None,
            ready: VecDeque::new(),
        }
    }
}

// ── 每核调度器：就绪队列操作 ──────────────────────────────────
//
// 队列长度与 READY_LENS 计数**恒同步**（入队 +1 / 出队 -1）——steal 依赖锁外
// 读该计数做空队列快速跳过，此处封装该不变量，杜绝单点失步。

impl Scheduler {
    /// 就绪入队（新任务 spawn / 抢占轮转）：队尾 + ready_lens 计数。
    fn enqueue(&mut self, task: Arc<Task>) {
        self.ready.push_back(task);
        ready_lens()[self.hart].0.fetch_add(1, Ordering::Relaxed);
    }

    /// 队首出队（tick / exit_current / enter_first_task / steal 共用）：出队 +
    /// ready_lens 计数；空队列返回 None。
    fn pop_ready(&mut self) -> Option<Arc<Task>> {
        let task = self.ready.pop_front()?;
        ready_lens()[self.hart].0.fetch_sub(1, Ordering::Relaxed);
        Some(task)
    }
}

// ── 每核调度器：运行与切换 ────────────────────────────────────

impl Scheduler {
    /// 任务即将在本 hart 上运行：置 Running + 写 kernel_sp（本 hart trap 栈顶——
    /// steal 迁移正确性的关键）+ 武装定时器。调用方须持本 hart 调度锁。
    fn prepare_resume(&self, task: &Arc<Task>) {
        task.transition(TaskState::Running);
        // SAFETY: 帧 PA 恒等映射可写；帧属 task 独占（current 或刚从就绪队列摘出）。
        unsafe {
            let frame = &mut *(task.trap.pa.as_usize() as *mut TrapContext);
            frame.kernel_sp = VirtAddr::from_raw(trap_stack_top(self.hart));
        }
        arm_timer(TIMER_INTERVAL);
    }

    /// 把任务装为本 hart 的 current 并返回待恢复帧 PA（steal 迁移 / 首次运行路径）。
    fn install_current(&mut self, task: Arc<Task>) -> usize {
        self.prepare_resume(&task);
        let pa = task.trap.pa.as_usize();
        self.current = Some(task);
        pa
    }

    /// 抢占/让出：当前任务轮转到就绪队尾，队首成为下一运行任务。
    ///
    /// 返回下一运行任务帧 PA。仅应在用户态陷阱中调用（内核态陷阱须恢复被中断的
    /// 内核上下文——切换会丢弃它；防御性判断见 trap.rs 定时器分支）。
    fn tick(&mut self) -> usize {
        let Some(cur) = self.current.take() else {
            // tick 只可能来自用户态陷阱（恒有 current）——防御性 panic
            panic!("tick with no current task on hart {}", self.hart);
        };
        if self.ready.is_empty() {
            // 本 hart 唯一任务：无需轮转，继续运行
            let pa = cur.trap.pa.as_usize();
            self.current = Some(cur);
            return pa;
        }
        cur.transition(TaskState::Ready);
        self.enqueue(cur);
        let next = self.pop_ready().expect("non-empty");
        let pa = next.trap.pa.as_usize();
        self.prepare_resume(&next);
        self.current = Some(next);
        pa
    }

    /// 当前线程退出：清理簿记 + 回收线程私有资源（栈 slot + trap 帧，帧随窗口子
    /// Map 归还），drop Arc<Task>（团队引用递减——最后一个线程退出 → Team/Space
    /// 归零回收）。
    ///
    /// 返回下一运行任务帧 PA；本核就绪队列空 → None（调用方转入 steal 循环）。
    /// 注意：返回后当前任务帧已失效（其帧页已归还），不得再解引用。
    fn exit_current(&mut self) -> Option<usize> {
        let exited = self.current.take().expect("no running task");
        debug_assert_eq!(
            exited.state(),
            TaskState::Running,
            "current task must be running"
        );
        putln!("task #{} '{}': exited", exited.id, exited.name);
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        exited.team.prune_tasks(&exited);
        // 锁内回收（层级 1 → Space.inner=2 → FRAME=5 合法）：栈 slot + trap 帧归还
        exited.team.space.task_reclaim(exited.id, exited.trap.va);
        drop(exited);
        EXITED.fetch_add(1, Ordering::Relaxed);
        self.pop_ready().map(|next| {
            let pa = next.trap.pa.as_usize();
            self.prepare_resume(&next);
            self.current = Some(next);
            pa
        })
    }
}

// ── 每核调度器：查询 ──────────────────────────────────────────

impl Scheduler {
    /// 当前运行任务 id（诊断用；无任务返回 usize::MAX）。
    fn current_task_id(&self) -> usize {
        self.current.as_ref().map(|t| t.id).unwrap_or(usize::MAX)
    }

    /// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
    ///
    /// 供 trap 缺页路径取当前空间（多核下取本 hart 的 current）。
    fn with_current_space<R>(&self, f: impl FnOnce(&Space) -> R) -> R {
        let task = self.current.as_ref().expect("no running task");
        f(&task.team.space)
    }
}

// ── 每核调度器表（按实际核数动态分配）───────────────────────────
//
// boot 时 init_schedulers 从 buddy 取，Box::leak 进 OnceLock——机器核数由 DTB
// 决定，不固定 MAX_HARTS 静态数组（MAX_HARTS=8 仅为编译期安全上限）。

/// 每 hart 调度锁（level 1；类型级，彼此不嵌套）。
static SCHEDULERS: OnceLock<&'static [SpinLock<Scheduler>]> = OnceLock::new();

/// 就绪队列长度（锁外只读信号：steal 先读后锁；写须在持对应调度器锁时进行）。
#[repr(align(64))]
struct ReadyLen(AtomicUsize);

static READY_LENS: OnceLock<&'static [ReadyLen]> = OnceLock::new();

/// per-hart 调度锁数组（与 hart 数同尺寸）。
fn schedulers() -> &'static [SpinLock<Scheduler>] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

/// 就绪队列长度数组（与 hart 数同尺寸）。
fn ready_lens() -> &'static [ReadyLen] {
    READY_LENS.get().expect("ready lens not initialized")
}

/// 按实际核数（DTB）动态分配 per-hart 调度器状态（boot 时调用**恰好一次**，
/// 先于任何调度器访问：spawn / trap / steal）。
pub fn init_schedulers() {
    let n = machine::hart_count();
    assert!(n > 0, "no harts");
    let sched: Box<[SpinLock<Scheduler>]> = (0..n)
        .map(|h| SpinLock::new(Scheduler::new(h)))
        .collect();
    let lens: Box<[ReadyLen]> = (0..n).map(|_| ReadyLen(AtomicUsize::new(0))).collect();
    assert!(
        SCHEDULERS.set(Box::leak(sched)).is_ok(),
        "schedulers double init"
    );
    assert!(
        READY_LENS.set(Box::leak(lens)).is_ok(),
        "ready lens double init"
    );
}

// ── 任务生命周期计数与停机 ────────────────────────────────────
//
// 全退出检测：EXITED == SPAWNED → halt_all（srst + wfi，AtomicBool 防双核
// 同时触发——后到者 wfi 等复位）。

/// 已创建 / 已退出任务计数（全退出检测：EXITED == SPAWNED → 停机）。
static SPAWNED: AtomicUsize = AtomicUsize::new(0);
static EXITED: AtomicUsize = AtomicUsize::new(0);
/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);
/// WFI 休眠 hart 位图（bit h = hart h 正阻塞在 WFI 等待唤醒；enqueue 后据此
/// 发 IPI——休眠核醒来后可 steal 新任务）。
static SLEEPING: AtomicUsize = AtomicUsize::new(0);

/// 全部任务是否已退出（SPAWNED > 0 防 boot 早期误判）。
fn all_exited() -> bool {
    let spawned = SPAWNED.load(Ordering::Relaxed);
    spawned > 0 && EXITED.load(Ordering::Relaxed) == spawned
}

/// 全部任务已退出：显式停机（srst；AtomicBool 防双核同时触发——后到者 wfi）。
fn halt_all() -> ! {
    if HALTING.swap(true, Ordering::AcqRel) {
        // 已有核触发停机：等待系统复位
        loop {
            unsafe { core::arch::asm!("wfi") };
        }
    }
    putln!("task: all tasks exited, system halted");
    let _ = ecall::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

// ── 对外 API（薄包装：取本核锁 → 委托 Scheduler 方法）────────────

/// 新任务入队收尾（create.rs spawn_thread 调用）：持本 hart 调度锁完成
/// 入簿（Team.tasks）+ 入队 + SPAWNED 计数。
///
/// 调用方已完成栈/帧分配与填帧——本函数只做队列侧收尾；锁内不调 space 方法
/// （层级 1 → 3 合法；Team.tasks 与 Space.inner 不嵌套，见 lock/mod.rs）。
pub(crate) fn enqueue(task: Arc<Task>) {
    let me = machine::hart_id();
    {
        let mut sch = schedulers()[me].lock();
        task.team.push_task(&task);
        sch.enqueue(task);
        SPAWNED.fetch_add(1, Ordering::Relaxed);
    }
    // 新任务出现：唤醒 WFI 休眠核（可 steal 取活）
    wake_sleepers();
}

/// 抢占/让出入口（trap 定时器分支调用）。
pub fn tick() -> usize {
    let me = machine::hart_id();
    schedulers()[me].lock().tick()
}

/// 当前线程退出入口（envcall ENV_EXIT 分支调用）：本核退出；队列空则转
/// steal 循环取活（拿不到就 spin）；全部任务退出 → halt_all。
pub fn exit_current() -> usize {
    let me = machine::hart_id();
    let mut sch = schedulers()[me].lock();
    if let Some(pa) = sch.exit_current() {
        return pa;
    }
    drop(sch);
    // 本 hart 无任务：阻塞获取（Idle → Running）；全部退出则停机
    acquire_next()
}

/// 当前运行任务 id（诊断用；无任务返回 usize::MAX）。
pub fn current_task_id() -> usize {
    let me = machine::hart_id();
    schedulers()[me].lock().current_task_id()
}

/// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
///
/// 供 trap 缺页路径取当前空间（多核下取本 hart 的 current）。
pub fn with_current_space<R>(f: impl FnOnce(&Space) -> R) -> R {
    let me = machine::hart_id();
    schedulers()[me].lock().with_current_space(f)
}

// ── 跨核偷取 ────────────────────────────────────────────────

/// 把偷来的任务装为本 hart 的 current（steal 迁移后运行路径）。
fn install_stolen(task: Arc<Task>) -> usize {
    let me = machine::hart_id();
    schedulers()[me].lock().install_current(task)
}

/// 非阻塞偷取：尝试从其它 hart 的就绪队列摘一个任务。
///
/// 先读 READY_LENS（锁外原子读，S 态共享不失效缓存行）——空队列不做 RMW，
/// 避免对受害者锁行乒乓（RMW 只在真有活时发生）；有活才 try_lock（失败即
/// 跳过——受害者忙时不等待，无锁序规则）。锁内 pop_ready 复查队列防竞态。
fn steal() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    let n = machine::hart_count();
    for v in 0..n {
        if v == me {
            continue;
        }
        if ready_lens()[v].0.load(Ordering::Relaxed) == 0 {
            continue;
        }
        let Some(mut sch) = schedulers()[v].try_lock() else {
            continue;
        };
        let Some(task) = sch.pop_ready() else {
            continue;
        };
        drop(sch);
        putln!("hart {me}: stole task #{} '{}' from hart {v}", task.id, task.name);
        return Some(task);
    }
    None
}

// ── 核休眠与唤醒（Idle 的阻塞点）──────────────────────────────────

/// 唤醒所有 WFI 休眠中的 hart（enqueue 后调用：新任务出现 → 睡核可 steal）。
///
/// SBI IPI 把目标核 SSIP 置位（sie.SSIP 已使能），目标核在 WFI 中挂起即被唤醒
/// ——全局 SIE=0，只唤醒不取中断。错误（如核已醒）尽力而为。
fn wake_sleepers() {
    let sleeping = SLEEPING.load(Ordering::Acquire);
    if sleeping == 0 {
        return;
    }
    // a0 = hart_mask（≤ 8 核，mask_base = 0）
    let _ = ecall::IpiCall::new(ecall::fid::Ipi::SendIpi)
        .args(SArgs {
            a0: sleeping,
            ..Default::default()
        })
        .call();
}

/// 本 hart 进入 WFI 休眠（Idle 自环的「阻塞点」）。
///
/// 协议（闭合「置位后漏唤醒」窗口）：置睡眠位 → **复查**（自核队首 / steal——
/// 置位前的 enqueue 已按位补 IPI，置位后的 enqueue 会看到位）→ 全退出检查 →
/// 停掉周期定时器（stimecmp 是本 hart 自己的 CSR，推远防 STIP 每量子唤醒）→
/// WFI。唤醒后清 SSIP 与睡眠位，返回 None 让外层循环重扫（也可能直接带出任务）。
fn idle_wait() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    SLEEPING.fetch_or(1usize << me, Ordering::AcqRel);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 enqueue 漏唤醒
    let found = {
        let mut sch = schedulers()[me].lock();
        match sch.pop_ready() {
            Some(task) => Some(task),
            None => {
                drop(sch);
                steal()
            }
        }
    };
    if let Some(task) = found {
        SLEEPING.fetch_and(!(1usize << me), Ordering::AcqRel);
        return Some(task);
    }
    if all_exited() {
        halt_all();
    }
    // 停掉周期定时器唤醒（stimecmp 推远）——**除非睡眠队列非空**：此时必须保持
    // tick，让任意核（含空闲核）都能跑 wake_due 唤醒到期任务（防全员阻塞死等）。
    if SLEEP_LIST.lock().is_empty() {
        crate::runtime::trap::arm_timer(1 << 60);
    }
    // WFI：SSIP（IPI）/ 残留 STIP 挂起即唤醒——只唤醒不取中断（SIE=0）
    unsafe {
        core::arch::asm!("wfi");
    }
    // 唤醒后清 SSIP（防残留位导致下次 WFI 立即重醒）与睡眠位
    unsafe {
        core::arch::asm!("csrc sip, 2");
    }
    SLEEPING.fetch_and(!(1usize << me), Ordering::AcqRel);
    None
}

/// Idle → Running 阻塞获取（enter_first_task / exit_current 空分支 / idle_loop
/// 共用的唯一转移）：自核队首 → 跨核 steal → 全退出检查 → WFI 休眠（IPI 唤醒）。
fn acquire_next() -> usize {
    let me = machine::hart_id();
    loop {
        {
            let mut sch = schedulers()[me].lock();
            if let Some(task) = sch.pop_ready() {
                return sch.install_current(task);
            }
        }
        if all_exited() {
            halt_all();
        }
        if let Some(task) = steal() {
            return install_stolen(task);
        }
        if let Some(task) = idle_wait() {
            return install_stolen(task);
        }
    }
}

// ── 任务阻塞：睡眠队列（Blocked 态的等待队列）──────────────────────

/// 睡眠条目：唤醒截止时间（time::read() 绝对时间）+ 阻塞任务。
struct SleepEntry {
    wake_at: usize,
    task: Arc<Task>,
}

/// 睡眠阻塞队列（FIFO；wake_at 单调递增——后睡者必后醒，队首即最早到期）。
///
/// 持锁纪律（见 lock/mod.rs，与 Team.tasks 同级）：block 路径「调度锁 → 队列
/// 锁」嵌套合法；wake 路径（wake_due）**先放队列锁再取调度锁**——绝不持队列锁
/// 取调度锁（防 ABBA）。
static SLEEP_LIST: SpinLock<VecDeque<SleepEntry>> = SpinLock::new(VecDeque::new());

/// 当前任务睡眠 `ticks` 个调度量子（Running → Blocked → 睡眠队列；到期由 trap
/// 定时器分支 wake_due 唤醒）。返回下一运行任务帧 PA（本核队列空则转阻塞获取）。
///
/// 调用方注意：返回后当前任务帧**仍有效**（阻塞非退出——帧未回收）。
pub fn sleep(ticks: usize) -> usize {
    let me = machine::hart_id();
    let wake_at = riscv::register::time::read() + ticks * TIMER_INTERVAL;
    let mut sch = schedulers()[me].lock();
    let task = sch.current.take().expect("no running task");
    debug_assert_eq!(
        task.state(),
        TaskState::Running,
        "current task must be running"
    );
    putln!(
        "task #{} '{}': sleeping {} ticks (wake @ {wake_at:#x})",
        task.id,
        task.name,
        ticks
    );
    task.transition(TaskState::Blocked);
    // 调度锁 → 睡眠队列锁（同层嵌套合法；wake 路径绝不反向嵌套）
    SLEEP_LIST.lock().push_back(SleepEntry { wake_at, task });
    match sch.pop_ready() {
        Some(next) => {
            let pa = next.trap.pa.as_usize();
            sch.prepare_resume(&next);
            sch.current = Some(next);
            pa
        }
        None => {
            drop(sch);
            acquire_next()
        }
    }
}

/// 唤醒入队（Blocked → Ready 后）：只入队 + 计数 + 唤醒休眠核；**不做**团队
/// 簿记（任务已在团队簿记中）与 SPAWNED（非新任务）。
fn wake_enqueue(task: Arc<Task>) {
    let me = machine::hart_id();
    {
        let mut sch = schedulers()[me].lock();
        sch.enqueue(task);
    }
    wake_sleepers();
}

/// tick 唤醒：扫描睡眠队列，到期任务 Blocked → Ready 出队并入调度队列（调用方
/// = trap 定时器分支；队列锁先放后取，绝不持队列锁取调度锁）。
pub fn wake_due() {
    let now = riscv::register::time::read();
    let due: Vec<Arc<Task>> = {
        let mut list = SLEEP_LIST.lock();
        let mut due = Vec::new();
        loop {
            let due_front = match list.front() {
                Some(entry) => entry.wake_at <= now,
                None => false,
            };
            if !due_front {
                break;
            }
            due.push(list.pop_front().expect("front checked").task);
        }
        due
    };
    for task in due {
        task.transition(TaskState::Ready);
        putln!("task #{} '{}': woken", task.id, task.name);
        wake_enqueue(task);
    }
}

// ── 核入口（首次进入调度与副核 idle）────────────────────────────

/// 调度入口（hart 0 首次进入）：从本 hart 就绪队列取首任务装为 current 并返回
/// 帧 PA；队列空则 steal（与 exit_current 空分支同语义）。
///
/// 不能用 spawn 返回的帧 PA 直接 restore：start_secondary_harts 之后副核可能已
/// 把首任务 steal 走——那个 PA 已过期（任务在别核运行），直接恢复会双核跑同一
/// 任务 + 本核 current 恒 None。
pub fn enter_first_task() -> usize {
    // 与 exit_current 空分支 / idle_loop 共用同一 Idle→Running 获取转移
    acquire_next()
}

/// 副核 idle 循环：spin + steal；拿到任务即 restore（永不返回）；全退出停机。
pub fn idle_loop() -> ! {
    loop {
        let pa = acquire_next();
        restore(pa)
    }
}
