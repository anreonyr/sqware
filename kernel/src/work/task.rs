// 线程（可调度单元）— 类型 + 构造。
//
// Task = 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
// TaskBuilder 在团队容器内生成任务：栈 + trap 帧 + 填帧 + 入队。
// 程序装载（loader.rs）与团队容器化（team.rs）在任务生成之前完成。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::space::{TASK_STACK_SIZE, kernel_frame_pa};
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::trap_stack_top;
use crate::work::USER_TEXT_BASE;

use super::scheduler;
use super::team::Team;

/// 全局任务号（跨 hart 唯一）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// 任务状态：任务现在在哪 +（Running/Blocked 时）该状态特有的数据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 正在执行（恒为某 hart 的 running，不在任何队列）：预算随 run 递减。
    /// 不变量：预算恒 ≥ 1（耗尽即转 Starved，不落盘 Running{0}）。
    Running { ticks_left: u32 },
    /// 已阻塞（在 blocked 容器中；不在任何就绪队列，不可被 steal）：原因在载荷。
    Blocked { reason: BlockReason },
    /// 已饥饿（预算耗尽，在 starved 容器等补给；被选中时重置满额预算）。
    Starved,
    /// 已收割（僵尸，在 reaped 容器等延迟回收；不在任何队列，任何核可回收）。
    Reaped,
}

/// Blocked 的载荷：阻塞原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// 睡眠：wake_at（timebase 刻度）到期由 unpark 唤醒。
    Park { wake_at: u64 },
}

/// trap 帧句柄 — 线程 trap 帧的薄引用。
///
/// 帧页由所属 Space 的 Frame 窗口子 Map **持有**（随线程退出回收），本句柄只
/// 携带 VA/PA 两个数：PA 供 restore 直接取帧，VA 供退出时按位归还窗口。
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    /// 帧在本空间中的虚拟地址（Frame 窗口分配，S-only）。
    pub(crate) va: VirtAddr,
    /// 帧物理地址（restore 的 a0）。
    pub(crate) pa: PhysAddr,
}

/// 线程 — 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
///
/// 栈 / 堆 / 帧全部归 Team.space 的窗口簿记（Window 子 Map），Task 只持
/// trap 句柄与共享的 team 引用——无任何页所有权。
pub struct Task {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    /// 状态（含载荷）。普通字段：只有经 scheduler::task_mut 的 &mut 能改。
    pub(crate) state: TaskState,
    pub(crate) team: Arc<Team>,
    pub(crate) trap: TrapFrame,
}

impl Task {
    /// 状态变换（状态机不变量）：非法变换直接 panic。
    ///
    /// 合法变换：
    ///   Starved → Running（调度器选上 / steal 迁移后运行）
    ///   Running → Starved（预算耗尽轮转 / 主动让出）
    ///   Running → Blocked(原因)（阻塞：如睡眠）
    ///   Blocked(_) → Starved（唤醒：回到就绪容器）
    ///   Running → Reaped（退出：标记收割，延迟回收）
    pub(crate) fn transform(&mut self, next: TaskState) {
        let legal = matches!(
            (self.state, next),
            (TaskState::Starved, TaskState::Running { .. })
                | (TaskState::Running { .. }, TaskState::Starved)
                | (TaskState::Running { .. }, TaskState::Blocked { .. })
                | (TaskState::Blocked { .. }, TaskState::Starved)
                | (TaskState::Running { .. }, TaskState::Reaped)
        );
        assert!(
            legal,
            "illegal task state transform: {:?} -> {:?}",
            self.state, next
        );
        self.state = next;
    }

    pub(crate) fn state(&self) -> TaskState {
        self.state
    }

    /// 续跑：预算递减（Running → Running 仅载荷更新，不经状态机变换表）。
    pub(crate) fn dec_ticks_left(&mut self) {
        match self.state {
            TaskState::Running { ticks_left } => {
                debug_assert!(ticks_left >= 1, "Running 预算恒 ≥ 1");
                self.state = TaskState::Running {
                    ticks_left: ticks_left - 1,
                };
            }
            _ => unreachable!("dec_ticks_left 只对 Running 任务调用"),
        }
    }
}

/// 任务构建器：在团队容器内生成线程（栈 + trap 帧 + 填帧 + 入队）。
///
/// 入口参数 arg 写入用户上下文 a0（blob D 按其分支行为）。空间分配（栈/帧）
/// 在调度器锁外完成（id 已原子化、空间自有锁）——锁只保护本 hart 队列的
/// push（与偷取者的 pop 互斥）与入簿（1 → 3 合法）。
///
/// # Errors
///
/// 栈/帧分配失败（MapError 原样传播）；失败时已分配资源随 Space drop 回滚。
pub struct TaskBuilder {
    team: Arc<Team>,
    name: &'static str,
    entry: VirtAddr,
    arg: usize,
}

impl TaskBuilder {
    /// 在指定团队内生成任务。
    pub fn new(team: Arc<Team>) -> TaskBuilder {
        TaskBuilder {
            team,
            name: "task",
            entry: USER_TEXT_BASE,
            arg: 0,
        }
    }

    /// 线程名（默认 "task"）。
    pub fn name(mut self, name: &'static str) -> TaskBuilder {
        self.name = name;
        self
    }

    /// 线程入口参数（写入用户上下文 a0）。
    pub fn arg(mut self, arg: usize) -> TaskBuilder {
        self.arg = arg;
        self
    }

    /// 线程入口（loader 产出的绝对 entry；默认 USER_TEXT_BASE）。
    pub fn entry(mut self, entry: VirtAddr) -> TaskBuilder {
        self.entry = entry;
        self
    }

    /// 生成任务：栈 slot + trap 帧（入团队空间窗口簿记）→ 填帧 → 入队收尾。
    /// 返回新线程 trap 帧 PA。
    pub fn spawn(self) -> Result<PhysAddr, MapError> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let me = machine::hart_id();

        // 1. 栈：Stack 窗口 slot（守护页 + 栈体子 Map，owner = id）→ 分配帧 attach
        let stack_va = self.team.space.stack_allocate(id)?;
        let mut stack_frames = Vec::new();
        for _ in 0..(TASK_STACK_SIZE / PAGE_SIZE) {
            let frame = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            stack_frames.push(frame);
        }
        self.team.space.stack_attach(stack_va, stack_frames)?;
        let stack_top = stack_va + TASK_STACK_SIZE;

        // 2. trap 帧：Frame 窗口取一页 VA + 物理帧 + 映射（S-only，owner = id）
        let (frame_va, frame_pa) = self.team.space.frame_allocate(id)?;

        // 3. 填帧：内核切换元数据从内核帧拷贝；用户上下文 = 入口/栈顶/a0/状态。
        //    kernel_sp = **本 hart** trap 栈顶（任务随后在本 hart 首次运行；若被
        //    steal 走，偷取核会在上台前重写——见 scheduler::prepare）。
        unsafe {
            let ktc = kernel_frame_pa(0).as_usize() as *const TrapContext;
            let frame = &mut *(frame_pa.as_usize() as *mut TrapContext);
            frame.kernel_satp = (*ktc).kernel_satp;
            frame.kernel_sp = VirtAddr::from_raw(trap_stack_top(me));
            frame.trap_handler = (*ktc).trap_handler;
            frame.trap_stack_corrupt = (*ktc).trap_stack_corrupt;
            frame.user_pa = frame_pa;
            // user_satp = Sv39 模式位(8) << 60 | asid << 44 | root_ppn —— restore 切回本空间用
            frame.user_satp =
                (8usize << 60) | (self.team.space.asid() << 44) | self.team.space.root();
            frame.self_va = frame_va.as_usize();
            frame.sepc = self.entry.as_usize();
            frame.gpr[2] = stack_top.as_usize();
            frame.gpr[10] = self.arg;
            let s = riscv::register::sstatus::read().bits();
            frame.sstatus = (s & !(1 << 1) & !(1 << 8)) | (1 << 5);
        }

        // 4. 入队收尾（初始状态 Starved；持本 hart 调度锁完成入簿 + 入队 + 计数）
        let task = Arc::new(Task {
            id,
            name: self.name,
            state: TaskState::Starved, // 初始就绪：入 starved 容器等首次选中（上台重置满额预算）
            team: self.team.clone(),
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
        scheduler::push(task);
        Ok(frame_pa)
    }
}
