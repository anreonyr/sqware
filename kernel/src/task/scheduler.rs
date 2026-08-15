// 任务调度器 — round-robin 轮转（S-timer 抢占 + envcall 主动让出）
//
// 队列模型：`VecDeque<Task>`，队首恒为当前运行任务。切换只在 trap 边界发生
// （内核态恒关中断，见 runtime/trap.rs 的 SIE 策略——内核代码永不抢占/阻塞）
//   tick()               — 抢占/让出：当前任务轮转到队尾，队首成为下一运行任务
//   exit_current()       — 当前任务退出：出队并在锁内回收（Space/栈帧/ASID）
//   with_current_space() — 闭包形式在锁内借出当前任务 Space（引用不逃逸锁）
//
// 切换语义：`tick`/`exit_current` 返回下一任务 trap 帧 PA；trap_handler 原样
// 返回该 PA，trampoline 尾部 `j __restore` 用 a0 = 该 PA 恢复——单任务帧轮转，
// 无独立切换汇编（见 runtime/trampoline.rs）。任务帧 PA 经 `Space::translate`
// （TRAP_CONTEXT VA 页表读路径）现取，不再冗余存于 Task。
//
// 锁层级（lock/mod.rs）：SCHEDULER = 层级 1（与 KERNEL_SPACE 同级；二者不
// 嵌套——spawn 的空间构建在锁外完成）。锁内允许获取层级 > 1 的锁
// （Space.inner=2、ASID=3、FRAME=4）；`exit_current` 在锁内 drop Task
// （Space::drop → ASID/帧归还）即 1 → {3,4}，合法；`frame_pa` 持 SCHEDULER
// 取 Space.inner（1 → 2）亦合法。禁止同 hart 重入 SCHEDULER（SpinLock 检测并
// panic）。
//
// 队列空（全部任务退出）：打印后 wfi 停机。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use crate::lock::SpinLock;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::PhysAddr;
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{MapKind, Space, SpaceBuilder, TASK_STACK_SIZE, TRAP_CONTEXT};
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::task::USER_TEXT_BASE;

/// 任务状态。
///
/// 阶段 C 退出即回收（exit_current 出队并 drop），不保留 Exited 实体，
/// 故只有 Ready / Running 两态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 就绪（在调度队列中等待运行）。
    Ready,
    /// 当前运行（恒为队列队首）。
    Running,
}

/// 任务 — 可调度单元：独立地址空间 + 调度状态。
///
/// 所有权：`space` 随 Task 一并 drop（exit 时回收）。栈/文本/trap 帧全部
/// 归 Space 的簿记（Window 子 Map / 常数 Map），Task 不再持有任何帧字段。
pub struct Task {
    id: usize,
    name: &'static str,
    state: TaskState,
    space: Space,
}

/// 调度器状态 — 就绪队列（队首 = 当前运行任务）+ 任务号。
pub struct Scheduler {
    queue: VecDeque<Task>,
    next_id: usize,
}

static SCHEDULER: SpinLock<Scheduler> = SpinLock::new(Scheduler {
    queue: VecDeque::new(),
    next_id: 1,
});

/// 任务的 trap-context 帧物理地址（`__restore` 目标）。
///
/// 经 `Space::translate` 现取（TRAP_CONTEXT 为每空间固定 VA，页表读路径；
/// 任务构建时已映射）。调用方须持 SCHEDULER 锁（1 → Space.inner=2，合法）。
fn frame_pa(task: &Task) -> usize {
    task.space
        .translate(TRAP_CONTEXT)
        .expect("running task has trap context mapped")
        .0
        .as_usize()
}

/// 生成一个新任务：建空间 → 映射文本/栈/trap 帧 → 填 trap 帧 → 入就绪队列。
///
/// 返回新任务 trap 帧物理地址（供 init 进入首任务）。程序须 ≤ 1 页
/// （阶段 C blob 自检；ELF 加载预留同基址）。
///
/// # Errors
///
/// 空间构建/映射/帧分配失败（`MapError` 原样传播）。
pub fn spawn(program: &'static [u8], name: &'static str) -> Result<PhysAddr, MapError> {
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

    // 2. 栈：Stack 窗口 slot（守护页 Reserved 子 Map + 栈体 Anonymous 子 Map）
    //    → 分配 4 帧 attach（帧入栈体子 Map，随 Space drop 回收）
    let stack_va = space.stack_alloc()?;
    let mut stack_frames = Vec::new();
    for _ in 0..(TASK_STACK_SIZE / PAGE_SIZE) {
        let frame =
            Box::try_new_in([0u8; PAGE_SIZE], allocator()).map_err(|_| MapError::OutOfMemory)?;
        stack_frames.push(frame);
    }
    space.stack_attach(stack_va, stack_frames)?;
    let stack_top = stack_va + TASK_STACK_SIZE;

    // 3. trap 帧（seed_user 已写 kernel_satp/kernel_sp/trap_handler/user_satp/self_pa；
    //    补用户上下文：sepc = 入口，sp = 栈顶，a0 = 0；sstatus SPP=0/SPIE=1/SIE=0）
    let (trap_ctx_pa, _) = space.translate(TRAP_CONTEXT).ok_or(MapError::NotMapped)?;
    let frame = unsafe { &mut *(trap_ctx_pa.as_usize() as *mut TrapContext) };
    let s = riscv::register::sstatus::read().bits();
    frame.sepc = USER_TEXT_BASE.as_usize();
    frame.gpr[2] = stack_top.as_usize();
    frame.gpr[10] = 0;
    frame.sstatus = (s & !(1 << 1) & !(1 << 8)) | (1 << 5);

    // 4. 入队（空间构建在锁外完成——SCHEDULER 与 KERNEL_SPACE 不嵌套）
    let mut sch = SCHEDULER.lock();
    let id = sch.next_id;
    sch.next_id += 1;
    putln!(
        "task #{id} '{name}': spawned, frame @ {:#x}, stack @ {:#x}",
        trap_ctx_pa.as_usize(),
        stack_top.as_usize()
    );
    sch.queue.push_back(Task {
        id,
        name,
        state: TaskState::Ready,
        space,
    });
    Ok(trap_ctx_pa)
}

/// 抢占/让出：轮转就绪队列，返回下一运行任务帧 PA。
///
/// 仅应在用户态陷阱中调用（内核态陷阱须恢复被中断的内核上下文——切换会丢弃
/// 它；防御性判断见 trap.rs 定时器分支）。
pub fn tick() -> usize {
    let mut sch = SCHEDULER.lock();
    if sch.queue.len() <= 1 {
        // 单任务：无需轮转，直接返回其帧
        return frame_pa(sch.queue.front().expect("queue empty"));
    }
    let mut cur = sch.queue.pop_front().expect("queue empty");
    cur.state = TaskState::Ready;
    sch.queue.push_back(cur);
    let next = sch.queue.front_mut().expect("queue empty");
    next.state = TaskState::Running;
    frame_pa(next)
}

/// 当前任务退出：出队并在锁内回收（Space/栈帧/ASID），返回下一运行任务帧 PA。
///
/// 调用方注意：返回后当前任务帧已失效（其空间已回收），不得再解引用。
pub fn exit_current() -> usize {
    let mut sch = SCHEDULER.lock();
    let exited = sch.queue.pop_front().expect("no running task");
    putln!("task #{} '{}': exited", exited.id, exited.name);
    // 锁内 drop（层级 1 → {3,4} 合法，见模块注释）：Space::drop 归还 ASID
    // （含 sfence）+ 页表/数据帧/栈帧（durable + 窗口子 Map 帧随字段 drop）。
    drop(exited);
    match sch.queue.front_mut() {
        Some(next) => {
            next.state = TaskState::Running;
            frame_pa(next)
        }
        None => {
            drop(sch);
            halt_all()
        }
    }
}

/// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
///
/// 供 trap 缺页路径取当前空间（阶段 B 的 `user::user_space` 由本函数取代）。
pub fn with_current_space<R>(f: impl FnOnce(&Space) -> R) -> R {
    let sch = SCHEDULER.lock();
    let task = sch.queue.front().expect("no running task");
    f(&task.space)
}

/// 全部任务已退出：显式停机（wfi 死循环；内核态 SIE=0，不会再被中断唤醒）。
fn halt_all() -> ! {
    putln!("task: all tasks exited, system halted");
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
