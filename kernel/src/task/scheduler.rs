// 任务调度器 — round-robin 轮转（S-timer 抢占 + envcall 主动让出）
//
// 线程模型（阶段 A）：Team（进程容器）→ Task（线程）两层。
//   Team   — 唯一 Space + 全部线程簿记（弱引用，无强环）；多个 Task 共享同一空间
//   Task   — 可调度单元：Arc<Team> + 自己的 trap 帧句柄（TrapFrame { va, pa }）
//
// 引用图无环：Task → Arc<Team>（强）、Scheduler → Arc<Task>（强）。Team 由它的
// 线程持有：spawn 返回的 Arc<Team> 只是构造期句柄，spawn 完线程即 drop；最后一个
// 线程退出 → Arc<Team> 归零 → Team/Space（ASID + 全部帧）自动回收。
//
// 队列模型：VecDeque<Arc<Task>>，队首恒为当前运行任务。切换只在 trap 边界发生
// （内核态恒关中断，见 runtime/trap.rs 的 SIE 策略——内核代码永不抢占/阻塞）
//   tick()               — 抢占/让出：当前任务轮转到队尾，队首成为下一运行任务
//   exit_current()       — 当前线程退出：回收栈 slot + trap 帧 → drop Arc<Task>
//   with_current_space() — 闭包形式在锁内借出当前任务 Space（引用不逃逸锁）
//
// 切换语义：tick/exit_current 返回下一任务 trap 帧 PA；trap_handler 原样
// 返回该 PA，trampoline 尾部 j __restore 用 a0 = 该 PA 恢复——单任务帧轮转，
// 无独立切换汇编（见 runtime/trampoline.rs）。帧 PA 直接存于 Task.trap（不再经
// Space::translate(TRAP_CONTEXT) 现取——用户空间已无固定 TRAP_CONTEXT 映射）。
//
// 锁层级（lock/mod.rs）：SCHEDULER = 层级 1（与 KERNEL_SPACE 同级；二者不
// 嵌套——空间构建在锁外完成）。锁内允许获取层级 > 1 的锁（Space.inner=2、
// ASID=3、FRAME=4）：spawn_thread 持 SCHEDULER 做栈/帧分配（1 → 2 → 4）、
// exit_current 在锁内 drop Arc<Task>（Space::drop → ASID/帧归还）即 1 → {3,4}，
// 均合法。禁止同 hart 重入 SCHEDULER（SpinLock 检测并 panic）。
//
// 队列空（全部任务退出）：打印后 wfi 停机。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::lock::SpinLock;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{
    MapKind, Space, SpaceBuilder, TASK_STACK_SIZE, kernel_trap_context,
};
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::task::USER_TEXT_BASE;

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
/// trap 句柄与共享的 team 引用——无任何页所有权。**无运行状态字段**：
/// 状态由就绪队列位置隐含（队首 = 当前运行，其余 = 就绪）——Arc<Task> 不可变，
/// 状态位图须内部可变性才能维护，属过度设计。
pub struct Task {
    id: usize,
    name: &'static str,
    team: Arc<Team>,
    trap: TrapFrame,
}

/// 团队（进程）— 共享地址空间的线程容器。
///
/// 无内部锁：全部可变状态（Space 内部）只在 SCHEDULER 锁内改动。
/// 线程成员表暂缓：&Arc<Team> 下无法追加弱引用（需内部可变性，与「无内部锁」
/// 冲突），多核阶段经就绪队列扫描派生。
pub struct Team {
    space: Space,
}

/// 调度器状态 — 就绪队列（队首 = 当前运行任务）+ 任务号。
pub struct Scheduler {
    ready: VecDeque<Arc<Task>>,
    next_id: usize,
}

static SCHEDULER: SpinLock<Scheduler> = SpinLock::new(Scheduler {
    ready: VecDeque::new(),
    next_id: 0,
});

/// 生成一个新团队（进程）：建空间 → 映射文本/TRAMPOLINE → 建首个线程。
///
/// 返回 (首线程 trap 帧 PA, 团队句柄)。调用方 spawn 完线程后应 drop 句柄
/// （团队由它的线程持有，句柄只是构造期借用）。程序须 ≤ 1 页
/// （阶段 C blob 自检；ELF 加载预留同基址）。
///
/// # Errors
///
/// 空间构建/映射/帧分配失败（MapError 原样传播）。
pub fn spawn(program: &'static [u8], name: &'static str) -> Result<(PhysAddr, Arc<Team>), MapError> {
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
    let team = Arc::new(Team { space });

    // 3. 首线程（入口参数 a0 = 0）
    let first = spawn_thread(&team, name, 0)?;
    Ok((first, team))
}

/// 在团队内生成一个新线程：栈 slot + trap 帧（均入团队空间的窗口簿记，
/// owner = 线程 id）→ 填帧 → 入就绪队列。返回新线程 trap 帧 PA。
///
/// arg 写入用户上下文 a0（线程入口参数——blob D 按其分支行为）。
///
/// # Errors
///
/// 栈/帧分配失败（MapError 原样传播）；失败时已分配资源回滚。
pub fn spawn_thread(team: &Arc<Team>, name: &'static str, arg: usize) -> Result<PhysAddr, MapError> {
    // 空间构建在锁外完成——SCHEDULER 与 KERNEL_SPACE 不嵌套
    let mut sch = SCHEDULER.lock();
    let id = sch.next_id;
    sch.next_id += 1;

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

    // 3. 填帧：内核切换元数据从内核帧拷贝；用户上下文 = 入口/栈顶/a0/状态
    //    （self_va 让 alltraps/restore 经任意帧 VA 定位本帧——机制前提）
    // SAFETY: 内核帧 PA 恒等可读（trap::init 已写入元数据）；新帧独占。
    unsafe {
        let ktc = kernel_trap_context().as_usize() as *const TrapContext;
        let frame = &mut *(frame_pa.as_usize() as *mut TrapContext);
        frame.kernel_satp = (*ktc).kernel_satp;
        frame.kernel_sp = (*ktc).kernel_sp;
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

    // 4. 入队
    putln!(
        "task #{} '{}': spawned, frame @ {:#x}, stack @ {:#x}",
        id,
        name,
        frame_pa.as_usize(),
        stack_top.as_usize()
    );
    sch.ready.push_back(Arc::new(Task {
        id,
        name,
        team: team.clone(),
        trap: TrapFrame {
            va: frame_va,
            pa: frame_pa,
        },
    }));
    Ok(frame_pa)
}

/// 抢占/让出：轮转就绪队列，返回下一运行任务帧 PA。
///
/// 仅应在用户态陷阱中调用（内核态陷阱须恢复被中断的内核上下文——切换会丢弃
/// 它；防御性判断见 trap.rs 定时器分支）。
pub fn tick() -> usize {
    let mut sch = SCHEDULER.lock();
    if sch.ready.len() <= 1 {
        // 单任务：无需轮转，直接返回其帧
        return sch.ready.front().expect("queue empty").trap.pa.as_usize();
    }
    let cur = sch.ready.pop_front().expect("queue empty");
    sch.ready.push_back(cur);
    sch.ready.front().expect("queue empty").trap.pa.as_usize()
}

/// 当前线程退出：回收线程私有资源（栈 slot + trap 帧，帧随窗口子 Map 归还），
/// drop Arc<Task>（团队引用递减——最后一个线程退出 → Team/Space 归零回收），
/// 返回下一运行任务帧 PA。
///
/// 调用方注意：返回后当前任务帧已失效（其帧页已归还），不得再解引用。
pub fn exit_current() -> usize {
    let mut sch = SCHEDULER.lock();
    let exited = sch.ready.pop_front().expect("no running task");
    putln!("task #{} '{}': exited", exited.id, exited.name);
    // 锁内回收（层级 1 → Space.inner=2 → FRAME=4 合法，见模块注释）：
    // 先摘窗口子 Map（栈/帧页归还 frame 池 + VA 回位图），再 drop Arc<Task>
    // —— Space 在还有其它线程存活时保持完好。
    exited.team.space.task_reclaim(exited.id, exited.trap.va);
    drop(exited);
    match sch.ready.front() {
        Some(next) => next.trap.pa.as_usize(),
        None => {
            drop(sch);
            halt_all()
        }
    }
}

/// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
///
/// 供 trap 缺页路径取当前空间（阶段 B 的 user::user_space 由本函数取代）。
pub fn with_current_space<R>(f: impl FnOnce(&Space) -> R) -> R {
    let sch = SCHEDULER.lock();
    let task = sch.ready.front().expect("no running task");
    f(&task.team.space)
}

/// 全部任务已退出：显式停机（wfi 死循环；内核态 SIE=0，不会再被中断唤醒）。
fn halt_all() -> ! {
    putln!("task: all tasks exited, system halted");
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
