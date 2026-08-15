// 进程/线程创建（阶段 A 线程模型）：spawn（团队 + 空间 + 首线程）与
// spawn_thread（栈 slot + trap 帧 + 填帧 + 入队收尾）。
//
// 本模块只管「造」——空间构建、栈/帧分配、用户上下文填充；队列侧收尾
// （入簿 + 入队 + SPAWNED 计数）交给 scheduler::enqueue，锁纪律集中在
// scheduler.rs。阶段 C ELF 加载（沿用 USER_TEXT_BASE 基址）扩展点在此。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::lock::SpinLock;
use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{MapKind, SpaceBuilder, TASK_STACK_SIZE, kernel_trap_context};
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::trap_stack_top;
use crate::task::USER_TEXT_BASE;

use super::scheduler;
use super::task::{Task, TaskState, Team, TrapFrame};

/// 全局任务号（跨 hart 唯一；替代阶段 A 的锁内递增）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

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
/// owner = 线程 id）→ 填帧 → 入队收尾（scheduler::enqueue）。返回新线程
/// trap 帧 PA。
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
    //    steal 走，偷取核会在 resume 前重写——见 scheduler::prepare_resume）。
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

    // 4. 入队收尾（初始状态 Ready；持本 hart 调度锁完成入簿 + 入队 + 计数）
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
    putln!(
        "task #{} '{}': spawned ({:?}), frame @ {:#x}, stack @ {:#x}",
        task.id,
        task.name,
        task.state(),
        frame_pa.as_usize(),
        stack_top.as_usize()
    );
    scheduler::enqueue(task);
    Ok(frame_pa)
}
