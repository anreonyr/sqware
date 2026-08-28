// 线程（可调度单元）— 类型 + 构造。
//
// Task = 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
// TaskBuilder 在团队容器内生成任务：栈 + trap 帧 + 填帧 + 入队。

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::layout::{HART_FRAME_BASE, IMAGE_BASE, TASK_STACK_SIZE};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::runtime::switcher::context::TrapContext;
use crate::work::unit::space::SpaceKind;
use crate::work::unit::team::kernel;

use super::team::Team;
use crate::work::room::conductor;

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
    /// 睡眠：wake_at（timebase 刻度）到期后被唤醒。
    Park { wake_at: u64 },
    /// 事件等待：被 `wake(key)` 唤醒；有 wake_at 时也可到期唤醒（None = 永久）。
    Wait { wake_at: Option<u64> },
}

/// trap 帧句柄 — 线程 trap 帧的薄引用。
///
/// 帧页由所属 Space 的 Frame 窗口子 Map **持有**（随线程退出回收），本句柄只
/// 携带 VA/PA 两个数：PA 供直接取帧，VA 供退出时按位归还窗口。
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
/// 不可变身份（TaskIdent）与状态——无任何页所有权。身份可自由 clone（不影响
/// `Arc<Task>` 的 strong_count，exclusive 纪律见下）；状态唯一可变。
pub struct Task {
    /// 不可变身份（spawn 时定型；clone 它不影响本 Task 的强持有计数）。
    pub(crate) ident: Arc<TaskIdent>,
    /// 状态（含载荷）。唯一可变字段：只有经 [`Task::exclusive`] 的 &mut 能改（唯一
    /// 强持有语义见 exclusive）。
    pub(crate) state: TaskState,
}

/// 不可变身份：spawn 时定型；任何人自由 clone，无需任何锁。
///
/// 帧存活不变量：持 `Arc<TaskIdent>` **不保** `trap` 指向的帧页存活（帧归 Space
/// 窗口，退出由 `clear` 按 va 归还）。仅两个安全窗口使用 `trap`：同 hart trap
/// 内（顺序执行，帧必活）；崩溃现场（全核冻结，无并发回收）。
pub(crate) struct TaskIdent {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
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

    /// 唯一强持有下取 &mut（`Arc::get_mut` 的 weak ≥ 1 变体：每个任务 spawn 时
    /// 即被 `Team::push_task` 记入簿记（`Arc::downgrade`），weak_count ≥ 1 永不
    /// 归零，`Arc::get_mut` 恒失败。簿记弱引用**从不读 Task 字段**（只 downgrade /
    /// `ptr_eq` 比较），不构成可变访问冲突）。
    ///
    /// 调用方义务（约束所在）：任务任一时刻只被一个容器强持有（running /
    /// starved / blocked / reaped 恰好其一）→ strong == 1；互斥 = 锁 + 唯一强
    /// 持有，无需原子字段。debug 断言兜底（违规即 panic，含 task id/name）。
    pub(crate) fn exclusive(t: &mut Arc<Self>) -> &mut Task {
        #[cfg(debug_assertions)]
        assert_eq!(
            Arc::strong_count(t),
            1,
            "task #{} '{}': not uniquely held (strong_count != 1)",
            t.ident.id,
            t.ident.name
        );
        // SAFETY: strong == 1 ⇒ 无并发 &mut（互斥由调度器锁 + 计数保证）；
        // Team 簿记弱引用不读字段。等价 Arc::get_mut（其要求 weak == 0）。
        unsafe { &mut *Arc::as_ptr(t).cast_mut() }
    }
}

/// 任务构建器：在团队容器内生成线程（栈 + trap 帧 + 填帧 + 入队）。
///
/// 入口参数 arg 写入用户上下文 a0。空间分配（栈/帧）
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
    /// 栈体大小（页对齐；缺省 `TASK_STACK_SIZE`）。
    stack: usize,
}

impl TaskBuilder {
    /// 在指定团队内生成任务。
    pub fn new(team: Arc<Team>) -> TaskBuilder {
        TaskBuilder {
            team,
            name: "task",
            entry: IMAGE_BASE,
            arg: 0,
            stack: TASK_STACK_SIZE,
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

    /// 线程入口（绝对 entry；默认 IMAGE_BASE）。
    pub fn entry(mut self, entry: VirtAddr) -> TaskBuilder {
        self.entry = entry;
        self
    }

    /// 自定义栈体大小（页对齐向上取整；缺省 `TASK_STACK_SIZE`）。栈窗 slot
    /// 按此大小 fall 取段（自窗口顶向下排）。
    pub fn stack(mut self, size: usize) -> TaskBuilder {
        self.stack = size.max(1).next_multiple_of(PAGE_SIZE);
        self
    }

    /// 统一闭包式任务生成：团队 + 闭包建任务（闭包装箱
    /// → trampoline → 新任务栈上调用）。团队身份决定运行世界：kernel 团队 → S 态内核任务
    /// （内核堆装箱、入口 `ktask_trampoline`、SPP=1 由 spawn 按团队身份自动定）。
    /// 当前仅支持 kernel 团队（U 态用户闭包未接入）。
    ///
    /// 约束：`FnOnce + Send + 'static`——闭包可捕获、可搬移到新执行上下文。
    /// 内核任务运行于 SIE=1（帧 SPIE=1），可被 S-timer 抢占（现场经 persist 保全），
    /// 也可经 `conductor::ktask` 自愿让出/睡眠——忙等不返回则独占所在核。
    ///
    /// 闭包内可调用统一调度服务面 `conductor::ktask::{park, starve, reap}`：
    /// 与用户任务同帧 ABI 的自愿切换（软陷阱），唤醒后闭包在调用点继续。
    pub fn closure<F>(self, f: F) -> Result<usize, MapError>
    where
        F: FnOnce() + Send + 'static,
    {
        debug_assert!(
            matches!(self.team.space.kind(), SpaceKind::Kernel),
            "TaskBuilder::closure 目前仅支持 kernel 团队（内核态任务）"
        );
        // 双装箱：`Box<dyn FnOnce()>` 是胖指针不能直接转 usize，外包一层得薄指针
        let inner: Box<dyn FnOnce()> = Box::new(f);
        let holder: Box<Box<dyn FnOnce()>> = Box::new(inner);
        let ptr = Box::into_raw(holder) as usize;
        // SAFETY: 闭包在本地装箱，a0 传其薄指针；SPP=1 回 S 态运行于 `ktask_trampoline`。
        let entry = VirtAddr::from_raw(ktask_trampoline as *const () as usize);
        self.entry(entry).arg(ptr).spawn()
    }

    /// 生成任务：栈 slot + trap 帧（入团队空间窗口簿记）→ 填帧 → 入队收尾。
    /// 返回新任务号（全局唯一）。失败时已分配资源回滚（栈/帧窗口归还）。
    pub fn spawn(self) -> Result<usize, MapError> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

        // 栈：Stack 窗口 slot（守护页 + 栈体子 Map，owner = id）
        let stack_size = self.stack;
        let is_kernel = matches!(self.team.space.kind(), SpaceKind::Kernel);
        // 失败回滚：按 owner 摘栈 slot（帧随子 Map drop 归还）
        let rollback_stack = |space: &crate::work::unit::space::Space| {
            space.with_flush(|inner| {
                if let Some((slot_va, slot_size)) = inner.stack.reclaim(id) {
                    inner.durable.unmap_frames(slot_va, slot_size);
                }
            });
        };
        let stack_res = (|| {
            let stack_va = self
                .team
                .space
                .with(|inner| inner.stack.claim(id, stack_size, is_kernel))?;
            let mut stack_frames = Vec::new();
            for _ in 0..(stack_size / PAGE_SIZE) {
                let frame = unsafe {
                    Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                        .map_err(|_| MapError::OutOfMemory)?
                        .assume_init()
                };
                stack_frames.push(frame);
            }
            self.team
                .space
                .with_flush(|inner| inner.attach_dynamic(stack_va, stack_frames))?;
            Ok(stack_va + stack_size)
        })();
        let stack_top = match stack_res {
            Ok(top) => top,
            Err(e) => {
                // 栈 slot 已由 claim 登记（守护页/栈体子 Map）——失败回滚按 owner 摘整 slot。
                rollback_stack(&self.team.space);
                return Err(e);
            }
        };

        // trap 帧：Frame 窗口取一页 VA + 物理帧 + 映射（S-only，owner = id）
        let frame_res = self
            .team
            .space
            .with_flush(|inner| inner.frame.claim(&mut inner.durable, id));
        let (frame_va, frame_pa) = match frame_res {
            Ok(x) => x,
            Err(e) => {
                rollback_stack(&self.team.space);
                return Err(e);
            }
        };

        // 填帧：`TrapContext::init` 从 per-hart 帧模板拷元数据 + 用户上下文
        let frame = unsafe { &mut *(frame_pa.as_usize() as *mut TrapContext) };
        unsafe {
            let ktc = kernel()
                .expect("kernel team not initialized")
                .space
                .translate(HART_FRAME_BASE)
                .expect("kernel frame not mapped")
                .0
                .as_usize() as *const TrapContext;
            frame.init(
                &*ktc, &self.team, self.entry, stack_top, self.arg, frame_pa, frame_va,
            );
        }

        // 入队收尾
        let ident = Arc::new(TaskIdent {
            id,
            name: self.name,
            team: self.team.clone(),
            trap: TrapFrame {
                va: frame_va,
                pa: frame_pa,
            },
        });
        let task = Arc::new(Task {
            ident,
            state: TaskState::Starved,
        });
        conductor::task::push(task);
        Ok(id)
    }
}

/// 内核任务 trampoline：解包闭包、执行、跑完自动退出。
///
/// a0 = `Box<dyn FnOnce()>` 指针（`TaskBuilder::arg` 写入）。该函数作为内核任务的
/// sepc 入口，SPP=1 回 S 态执行于该任务内核栈上；闭包返回后退出调度。
///
/// 必须以 `-> !` 返回：从 `_start`-式入口返回会跳 0 崩溃，退出必须显式执行。
///
/// # Safety
/// `arg` 必须是对应闭包装箱（TaskBuilder::closure / kernel 侧）所产出的
/// `Box<dyn FnOnce()>` 原始指针。
pub(crate) extern "C" fn ktask_trampoline(arg: usize) -> ! {
    // tp = 本 hart PerHart 指针：每个内核任务上台时 Scheduler::prepare 已把 TP
    // 写入其帧（frame.gpr[TP] = per_hart_ptr(self.hart)），__restore 恢复全部 GPR
    // 时 tp 即已在位——此处不再重建。
    // SAFETY: arg 由 closure 以 Box::into_raw(holder) 产出（薄指针），此处独占回收。
    let holder: Box<Box<dyn FnOnce()>> = unsafe { Box::from_raw(arg as *mut Box<dyn FnOnce()>) };
    let boxed: Box<dyn FnOnce()> = *holder;
    boxed();
    conductor::ktask::reap()
}
