// 任务调度器 — 多核（hart B1）：per-hart 调度锁 + 非阻塞 steal + 动态 trap 栈
//
// 线程模型（阶段 A 延续）：Team（进程容器）→ Task（线程）两层。
//   Team   — 唯一 Space + 成员簿记（SpinLock<Vec<Weak<Task>>>，弱引用，无强环）
//   Task   — 可调度单元：Arc<Team> + 调度状态 + 自己的 trap 帧句柄（TrapFrame { va, pa }）
//
// 引用图无环：Task → Arc<Team>（强）、调度器 → Arc<Task>（强，current/ready）、
// Team → Weak<Task>（弱，簿记不参与生命周期）。Team 由它的线程持有：spawn 返回
// 的 Arc<Team> 只是构造期句柄，spawn 完线程即 drop；最后一个线程退出 → Arc<Team>
// 归零 → Team/Space（ASID + 全部帧）自动回收。
//
// 多核调度（锁模型 B）：
//   schedulers()[hart] — 每 hart 一把 SpinLock（level 1）：保护该 hart 的
//                      current（运行中，**不在队列**）+ ready（VecDeque）。
//   steal            — 跨 hart 取活：先读 ready_lens()[v]（锁外原子读，S 态共享
//                      不失效缓存行）再 try_lock（非阻塞）——空队列不做 RMW，
//                      避免对受害者锁行乒乓；拿不到即跳过（无锁序规则）。
//   idle             — 本 hart 无任务：spin + steal；全退出 → halt_all。
//
// 切换语义：tick/exit_current/steal 返回下一任务帧 PA；trap_handler 原样返回，
// trampoline 尾部 j __restore 用 a0 = PA 恢复。**kernel_sp 每切换写入**：把帧
// 交给 __restore 前写 frame.kernel_sp = trap_stack_top(hart_id())（__alltraps
// 从帧读 kernel_sp 上 per-hart trap 栈——steal 迁移后任务在偷取核上运行，写漏
// 会让它跑到旧核的栈上；debug assert 在 trap 入口兜底）。
//
// 锁层级（lock/mod.rs）：schedulers()[hart] = level 1（类型级，彼此不嵌套；
// try_lock steal 免于锁序规则）；Team.tasks = level 3——与 Space.inner = 2
// **禁止嵌套持有**：push_task/prune_tasks 是纯 Vec 操作，锁内绝不调 space 方法。
//
// 全退出：SPAWNED/EXITED 原子计数；EXITED == SPAWNED → halt_all（srst + wfi，
// AtomicBool 防双核同时触发）。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crate::ecall::fid;
use crate::lock::{OnceLock, SpinLock};
use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{
    MapKind, Space, SpaceBuilder, TASK_STACK_SIZE, kernel_trap_context,
};
use crate::runtime::context::TrapContext;
use crate::runtime::trap::{TIMER_INTERVAL, arm_timer};
use crate::runtime::trampoline::{restore, trap_stack_top};
use crate::task::USER_TEXT_BASE;
use crate::{ecall, putln};

/// 任务状态（生命周期：就绪 ↔ 运行）。
///
/// 存为 AtomicU8 而非裸枚举字段：Task 经 Arc 共享，状态转换须经不可变
/// 引用完成——原子字段是满足 Sync 的最小载体。实际全部转换都在调度器锁内
/// 发生（单写者 + 锁内读取），原子性只是形式而非并发协议。
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 就绪（在某 hart 的就绪队列中等待运行）。
    Ready = 0,
    /// 当前运行（恒为某 hart 的 current，不在任何队列）。
    Running = 1,
}

/// trap 帧句柄 — 线程 trap 帧的薄引用。
///
/// 帧页由所属 Space 的 Frame 窗口子 Map **持有**（随线程退出回收），本句柄只
/// 携带 VA/PA 两个数：PA 供 restore 直接取帧，VA 供退出时按位归还窗口。
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    /// 帧在本空间中的虚拟地址（Frame 窗口分配，S-only）。
    va: VirtAddr,
    /// 帧物理地址（restore 的 a0）。
    pa: PhysAddr,
}

/// 线程 — 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
///
/// 栈 / 堆 / 帧全部归 Team.space 的窗口簿记（Window 子 Map），Task 只持
/// trap 句柄与共享的 team 引用——无任何页所有权。
pub struct Task {
    id: usize,
    name: &'static str,
    state: AtomicU8,
    team: Arc<Team>,
    trap: TrapFrame,
}

impl Task {
    fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Relaxed);
    }

    fn state(&self) -> TaskState {
        match self.state.load(Ordering::Relaxed) {
            0 => TaskState::Ready,
            1 => TaskState::Running,
            // 只会写入 TaskState 的合法编码，此处不可能到达
            _ => unreachable!("invalid task state: {}", self.state.load(Ordering::Relaxed)),
        }
    }
}

/// 团队（进程）— 共享地址空间的线程容器。
///
/// tasks 为成员簿记（弱引用，无强环——线程由各 hart 的 current/就绪队列强
/// 持有），多核阶段用于团队视角的负载判断；生命周期仍由引用计数决定（最后
/// 一个线程退出 → Arc<Team> 归零 → 团队回收）。
///
/// 多核下 per-hart 调度锁不再提供跨 hart 互斥，故 tasks 自带 SpinLock
/// （level 3）。**不变量：持本锁时绝不调用任何 space 方法**——push_task /
/// prune_tasks 是纯 Vec 操作，与 Space.inner（level 2）只顺序获取、永不嵌套
/// 持有（ABBA 防御，见 lock/mod.rs 层级注释）。
pub struct Team {
    space: Space,
    /// 成员簿记（弱引用条目；死条目在下次清理时摘除）。
    tasks: SpinLock<Vec<Weak<Task>>>,
}

impl Team {
    /// 成员入簿（调用方通常持 schedulers()[hart]——spawn_thread；不调 space 方法）。
    fn push_task(&self, task: &Arc<Task>) {
        self.tasks.lock().push(Arc::downgrade(task));
    }

    /// 清理簿记：摘除已退出线程与全部死条目（弱引用无所有权，滞留仅占条目）。
    fn prune_tasks(&self, exited: &Arc<Task>) {
        self.tasks.lock().retain(|t| match t.upgrade() {
            // 已回收的死条目（strong_count == 0）与本线程条目一并摘除
            Some(a) => !Arc::ptr_eq(&a, exited),
            None => false,
        });
    }
}

/// 每 hart 调度器状态：current（运行中，不在队列）+ ready（就绪队列）。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub struct Scheduler {
    /// 当前运行任务（不在 ready 队列；steal 只从 ready 摘——current 不可被偷）。
    current: Option<Arc<Task>>,
    /// 就绪队列。
    ready: VecDeque<Arc<Task>>,
}

const fn new_scheduler() -> SpinLock<Scheduler> {
    SpinLock::new(Scheduler {
        current: None,
        ready: VecDeque::new(),
    })
}

/// 每 hart 调度锁（level 1；类型级，彼此不嵌套）。
///
/// **按实际核数动态分配**（boot 时 init_schedulers 从 buddy 取，Box::leak 进
/// OnceLock）——机器核数由 DTB 决定，不固定 MAX_HARTS 静态数组。
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
    let sched: Box<[SpinLock<Scheduler>]> = (0..n).map(|_| new_scheduler()).collect();
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

/// 全局任务号（跨 hart 唯一；替代阶段 A 的锁内递增）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// 已创建 / 已退出任务计数（全退出检测：EXITED == SPAWNED → 停机）。
static SPAWNED: AtomicUsize = AtomicUsize::new(0);
static EXITED: AtomicUsize = AtomicUsize::new(0);
/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);

// 实际活跃核数由 machine::hart_count() 提供（DTB 核数 ≤ MAX_HARTS）

/// 生成一个新团队（进程）：建空间 → 映射文本/TRAMPOLINE → 建首个线程。
///
/// 返回 (首线程 trap 帧 PA, 团队句柄)。调用方 spawn 完线程后应 drop 句柄
/// （团队由它的线程持有，句柄只是构造期借用）。程序须 ≤ 1 页
/// （阶段 C blob 自检；ELF 加载预留同基址）。
///
/// # Errors
///
/// 空间构建/映射/帧分配失败（MapError 原样传播）。
pub fn spawn(
    program: &'static [u8],
    name: &'static str,
) -> Result<(PhysAddr, Arc<Team>), MapError> {
    assert!(program.len() <= PAGE_SIZE, "task program exceeds one page");
    let space = SpaceBuilder::user().build()?;

    // 1. 文本：帧拷贝 blob → 常数 Map（R|X|U；帧归空间，随 Space drop 回收）
    let mut text =
        Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
    text[..program.len()].copy_from_slice(program);
    let text_pa = PhysAddr::from_raw(text.as_ptr() as usize);
    space.map(
        USER_TEXT_BASE,
        text_pa,
        PAGE_SIZE,
        PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::U | PteFlags::A | PteFlags::D,
        MapKind::Anonymous,
        vec![text],
    )?;

    // 2. 团队（由首线程持有；本函数返回构造期句柄）
    let team = Arc::new(Team {
        space,
        tasks: SpinLock::new(Vec::new()),
    });

    // 3. 首线程（入口参数 a0 = 0）
    let first = spawn_thread(&team, name, 0)?;
    Ok((first, team))
}

/// 在团队内生成一个新线程：栈 slot + trap 帧（均入团队空间的窗口簿记，
/// owner = 线程 id）→ 填帧 → 入簿 + 入队。返回新线程 trap 帧 PA。
///
/// arg 写入用户上下文 a0（线程入口参数——blob D 按其分支行为）。
///
/// 空间分配（栈/帧）在调度器锁外完成（id 已原子化、空间自有锁）——锁只保护
/// 本 hart 队列的 push（与偷取者的 pop 互斥）与入簿（1 → 3 合法）。
///
/// # Errors
///
/// 栈/帧分配失败（MapError 原样传播）；失败时已分配资源回滚。
pub fn spawn_thread(
    team: &Arc<Team>,
    name: &'static str,
    arg: usize,
) -> Result<PhysAddr, MapError> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let me = machine::hart_id();

    // 1. 栈：Stack 窗口 slot（守护页 + 栈体子 Map，owner = id）→ 分配 4 帧 attach
    let stack_va = team.space.stack_alloc(id)?;
    let mut stack_frames = Vec::new();
    for _ in 0..(TASK_STACK_SIZE / PAGE_SIZE) {
        let frame =
            Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
        stack_frames.push(frame);
    }
    team.space.stack_attach(stack_va, stack_frames)?;
    let stack_top = stack_va + TASK_STACK_SIZE;

    // 2. trap 帧：Frame 窗口取一页 VA + 物理帧 + 映射（S-only，owner = id）
    let (frame_va, frame_pa) = team.space.frame_alloc(id)?;

    // 3. 填帧：内核切换元数据从内核帧拷贝；用户上下文 = 入口/栈顶/a0/状态。
    //    kernel_sp = **本 hart** trap 栈顶（任务随后在本 hart 首次运行；若被
    //    steal 走，偷取核会在 resume 前重写——见 prepare_resume）。
    unsafe {
        let ktc = kernel_trap_context().as_usize() as *const TrapContext;
        let frame = &mut *(frame_pa.as_usize() as *mut TrapContext);
        frame.kernel_satp = (*ktc).kernel_satp;
        frame.kernel_sp = VirtAddr::from_raw(trap_stack_top(me));
        frame.trap_handler = (*ktc).trap_handler;
        frame.trap_stack_corrupt = (*ktc).trap_stack_corrupt;
        frame.user_pa = frame_pa;
        // user_satp = Sv39 模式位(8) << 60 | asid << 44 | root_ppn —— restore 切回本空间用
        frame.user_satp = (8usize << 60) | (team.space.asid() << 44) | team.space.root();
        frame.self_va = frame_va.as_usize();
        frame.sepc = USER_TEXT_BASE.as_usize();
        frame.gpr[2] = stack_top.as_usize();
        frame.gpr[10] = arg;
        let s = riscv::register::sstatus::read().bits();
        frame.sstatus = (s & !(1 << 1) & !(1 << 8)) | (1 << 5);
    }

    // 4. 入簿 + 入队（初始状态 Ready；持本 hart 调度锁）
    let task = Arc::new(Task {
        id,
        name,
        state: AtomicU8::new(TaskState::Ready as u8),
        team: team.clone(),
        trap: TrapFrame {
            va: frame_va,
            pa: frame_pa,
        },
    });
    let mut sch = schedulers()[me].lock();
    team.push_task(&task);
    putln!(
        "task #{} '{}': spawned ({:?}), frame @ {:#x}, stack @ {:#x}",
        task.id,
        task.name,
        task.state(),
        frame_pa.as_usize(),
        stack_top.as_usize()
    );
    sch.ready.push_back(task);
    ready_lens()[me].0.fetch_add(1, Ordering::Relaxed);
    SPAWNED.fetch_add(1, Ordering::Relaxed);
    Ok(frame_pa)
}

/// 任务即将在本 hart 上运行：置 Running + 写 kernel_sp（本 hart trap 栈顶——
/// steal 迁移正确性的关键）+ 武装定时器。调用方须持本 hart 调度锁。
fn prepare_resume(task: &Arc<Task>, hart: usize) {
    task.set_state(TaskState::Running);
    // SAFETY: 帧 PA 恒等映射可写；帧属 task 独占（current 或刚从就绪队列摘出）。
    unsafe {
        let frame = &mut *(task.trap.pa.as_usize() as *mut TrapContext);
        frame.kernel_sp = VirtAddr::from_raw(trap_stack_top(hart));
    }
    arm_timer(TIMER_INTERVAL);
}

/// 抢占/让出：本 hart 当前任务轮转到就绪队尾，队首成为下一运行任务。
///
/// 返回下一运行任务帧 PA。仅应在用户态陷阱中调用（内核态陷阱须恢复被中断的
/// 内核上下文——切换会丢弃它；防御性判断见 trap.rs 定时器分支）。
pub fn tick() -> usize {
    let me = machine::hart_id();
    let mut sch = schedulers()[me].lock();
    let Some(cur) = sch.current.take() else {
        // tick 只可能来自用户态陷阱（恒有 current）——防御性 panic
        panic!("tick with no current task on hart {me}");
    };
    if sch.ready.is_empty() {
        // 本 hart 唯一任务：无需轮转，继续运行
        let pa = cur.trap.pa.as_usize();
        sch.current = Some(cur);
        return pa;
    }
    cur.set_state(TaskState::Ready);
    sch.ready.push_back(cur);
    ready_lens()[me].0.fetch_add(1, Ordering::Relaxed);
    let next = sch.ready.pop_front().expect("non-empty");
    ready_lens()[me].0.fetch_sub(1, Ordering::Relaxed);
    let pa = next.trap.pa.as_usize();
    prepare_resume(&next, me);
    sch.current = Some(next);
    pa
}

/// 当前线程退出：清理簿记 + 回收线程私有资源（栈 slot + trap 帧，帧随窗口子
/// Map 归还），drop Arc<Task>（团队引用递减——最后一个线程退出 → Team/Space
/// 归零回收），返回下一运行任务帧 PA。
///
/// 本 hart 队列空且未全退出时：进入 steal 循环取活（拿不到就 spin）；全部
/// 任务退出 → halt_all。
///
/// 调用方注意：返回后当前任务帧已失效（其帧页已归还），不得再解引用。
pub fn exit_current() -> usize {
    let me = machine::hart_id();
    let mut sch = schedulers()[me].lock();
    let exited = sch.current.take().expect("no running task");
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
    match sch.ready.pop_front() {
        Some(next) => {
            ready_lens()[me].0.fetch_sub(1, Ordering::Relaxed);
            let pa = next.trap.pa.as_usize();
            prepare_resume(&next, me);
            sch.current = Some(next);
            pa
        }
        None => {
            drop(sch);
            // 本 hart 无任务：steal 直至拿到任务；全部退出则停机
            loop {
                if all_exited() {
                    halt_all();
                }
                if let Some(next) = steal() {
                    return install_current(next);
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// 把任务装为本 hart 的 current 并返回待恢复帧 PA（steal 路径用）。
fn install_current(task: Arc<Task>) -> usize {
    let me = machine::hart_id();
    prepare_resume(&task, me);
    let pa = task.trap.pa.as_usize();
    schedulers()[me].lock().current = Some(task);
    pa
}

/// 非阻塞偷取：尝试从其它 hart 的就绪队列摘一个任务。
///
/// 先读 READY_LENS（锁外原子读，S 态共享不失效缓存行）——空队列不做 RMW，
/// 避免对受害者锁行乒乓（RMW 只在真有活时发生）；有活才 try_lock（失败即
/// 跳过——受害者忙时不等待，无锁序规则）。锁内复查队列防读后变更竞态。
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
        if sch.ready.is_empty() {
            drop(sch);
            continue;
        }
        let task = sch.ready.pop_front().expect("non-empty");
        ready_lens()[v].0.fetch_sub(1, Ordering::Relaxed);
        drop(sch);
        putln!("hart {me}: stole task #{} '{}' from hart {v}", task.id, task.name);
        return Some(task);
    }
    None
}

/// 调度入口（hart 0 首次进入）：从本 hart 就绪队列取首任务装为 current 并返回
/// 帧 PA；队列空则 steal（与 exit_current 空分支同语义）。
///
/// 不能用 spawn 返回的帧 PA 直接 restore：start_secondary_harts 之后副核可能已
/// 把首任务 steal 走——那个 PA 已过期（任务在别核运行），直接恢复会双核跑同一
/// 任务 + 本核 current 恒 None。
pub fn enter_first_task() -> usize {
    let me = machine::hart_id();
    loop {
        let mut sch = schedulers()[me].lock();
        if let Some(task) = sch.ready.pop_front() {
            ready_lens()[me].0.fetch_sub(1, Ordering::Relaxed);
            let pa = task.trap.pa.as_usize();
            prepare_resume(&task, me);
            sch.current = Some(task);
            return pa;
        }
        drop(sch);
        if all_exited() {
            halt_all();
        }
        if let Some(task) = steal() {
            return install_current(task);
        }
        core::hint::spin_loop();
    }
}

/// 副核 idle 循环：spin + steal；拿到任务即 restore（永不返回）；全退出停机。
pub fn idle_loop() -> ! {
    loop {
        if all_exited() {
            halt_all();
        }
        if let Some(task) = steal() {
            let pa = install_current(task);
            restore(pa)
        }
        core::hint::spin_loop();
    }
}

/// 当前运行任务 id（诊断用；无任务返回 usize::MAX）。
pub fn current_task_id() -> usize {
    let me = machine::hart_id();
    let sch = schedulers()[me].lock();
    sch.current.as_ref().map(|t| t.id).unwrap_or(usize::MAX)
}

/// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
///
/// 供 trap 缺页路径取当前空间（多核下取本 hart 的 current）。
pub fn with_current_space<R>(f: impl FnOnce(&Space) -> R) -> R {
    let me = machine::hart_id();
    let sch = schedulers()[me].lock();
    let task = sch.current.as_ref().expect("no running task");
    f(&task.team.space)
}

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
