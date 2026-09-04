// 线程（可调度单元）— 类型 + 构造。
//
// Task = 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
// TaskBuilder 在团队容器内生成任务：栈 + trap 帧 + 填帧 + 入队。

use alloc::alloc::Allocator;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::layout::{HART_FRAME_BASE, IMAGE_BASE, TASK_STACK_SIZE};
use crate::lock::SpinLock;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::runtime::switcher::context::TrapContext;
use crate::work::mail::AnyPie;
use crate::work::unit::space::SpaceKind;
use crate::work::unit::space::window::{FrameWindow, StackWindow};
use crate::work::unit::team::kernel;

use super::team::Team;
use crate::work::room::scheduler;

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

/// 线程 — 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
///
/// 栈 / 帧全部归 Team.space 的映射簿记，Task 只持不可变身份（TaskIdent，含
/// 栈/帧的 [`Span`] 区间）与状态——无任何页所有权。身份可自由 clone（不影响
/// `Arc<Task>` 的 strong_count，exclusive 纪律见下）；状态唯一可变。
pub struct Task {
    /// 不可变身份（spawn 时定型；clone 它不影响本 Task 的强持有计数）。
    pub(crate) ident: Arc<TaskIdent>,
    /// 状态（含载荷）。唯一可变字段：只有经 [`Task::exclusive`] 的 &mut 能改（唯一
    /// 强持有语义见 exclusive）。
    pub(crate) state: TaskState,
    /// mail 门闩集合（每个门闩持 Arc<Meta>）。envcall 适配 push/pull，
    /// Task::drop 时 Arc 递减——最后 Arc drop 时 Meta 自然析构。锁级 = L3
    ///（与 messenger 簿记同级，绝不嵌套）。
    pub(crate) pies: SpinLock<Vec<AnyPie>>,
}

/// 不可变身份：spawn 时定型；任何人自由 clone，无需任何锁。
///
/// 资源存活不变量：持 `Arc<TaskIdent>` **不保** `stack`/`frame` 指向的映射存活
/// （映射归 Space，退役由 `clear` 经 [`Space::release`] 按 Span 归还）。仅两个
/// 安全窗口使用 `frame.pa`：同 hart trap 内（顺序执行，帧必活）；崩溃现场
/// （全核冻结，无并发回收）。
pub(crate) struct TaskIdent {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    pub(crate) team: Arc<Team>,
    /// 栈 slot 区间（user 段，pa=None）——回收经 [`Space::release`]。
    pub(crate) stack: crate::work::unit::space::Span,
    /// 帧（kernel 段，pa=Some）——restore 取帧、回收经 [`Space::release`]。
    pub(crate) frame: crate::work::unit::space::Span,
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
/// push（与偷取者的 pull 互斥）与入簿（1 → 3 合法）。
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
    /// 也可经 `scheduler::ktask` 自愿让出/睡眠——忙等不返回则独占所在核。
    ///
    /// 闭包内可调用统一调度服务面 `scheduler::ktask::{park, starve, reap}`：
    /// 与用户任务同帧 ABI 的自愿切换（软陷阱），唤醒后闭包在调用点继续。
    pub fn closure<F>(self, f: F) -> Result<usize, MapError>
    where
        F: FnOnce() + Send + 'static,
    {
        debug_assert!(
            matches!(self.team.space.kind(), SpaceKind::Kernel),
            "TaskBuilder::closure 目前仅支持 kernel 团队（内核态任务）"
        );
        // 双装箱：`Box<dyn FnOnce()>` 是胖指针不能直接转 usize，外包一层得薄指针。
        // 类别 = Task：闭包装箱属任务生命周期——关机 TASK_BLOCKS 归零（①）。
        // 装饰器标注（块侧：mark 默认 Persistent 后 relabel）；释放经地址路由 +
        // ledger 类别记账，不依赖分配器类型。
        let inner: Box<dyn FnOnce(), &'static dyn Allocator> = crate::tag!(
            Task,
            Box::new_in(f, crate::memory::allocator::block::allocator())
        );
        let holder: Box<Box<dyn FnOnce(), &'static dyn Allocator>, &'static dyn Allocator> = crate::tag!(
            Task,
            Box::new_in(inner, crate::memory::allocator::block::allocator())
        );
        // into_raw_with_allocator（非 Global 的 Box 无 into_raw）——alloc 是
        // 引用（drop 空操作），ptr 交 trampoline 的 Box::from_raw（Global 型，
        // 释放按地址路由 + ledger 类别记账）。
        let (ptr, _alloc) = Box::into_raw_with_allocator(holder);
        let ptr = ptr as usize;
        // SAFETY: 闭包在本地装箱，a0 传其薄指针；SPP=1 回 S 态运行于 `ktask_trampoline`。
        let entry = VirtAddr::from_raw(ktask_trampoline as *const () as usize);
        self.entry(entry).arg(ptr).spawn()
    }

    /// 生成任务：栈 slot + trap 帧（入团队空间窗口簿记）→ 填帧 → 入队收尾。
    /// 返回新任务号（全局唯一）。失败时已分配资源回滚（栈/帧经 `Space::release`）。
    pub fn spawn(self) -> Result<usize, MapError> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

        // 栈：StackWindow::claim 取 slot（user 段 + guard，立即物化）
        let stack_size = self.stack;
        let is_kernel = matches!(self.team.space.kind(), SpaceKind::Kernel);
        let stack_span = StackWindow::claim(&self.team.space, stack_size, is_kernel)?;
        // 栈体基址（供填帧算 stack_top）= slot 基址 + guard
        let stack_body = stack_span.va + crate::layout::TASK_STACK_GUARD;
        let stack_top = stack_body + stack_size;

        // trap 帧：FrameWindow::claim（kernel 段，立即物化）
        let frame_span = match FrameWindow::claim(&self.team.space) {
            Ok(s) => s,
            Err(e) => {
                // 栈已领——用局部 Span 回滚（不读 TaskIdent，此时未构造）
                self.team
                    .space
                    .release(stack_span)
                    .expect("release: rollback");
                return Err(e);
            }
        };
        let frame_pa = frame_span.pa.expect("frame span has pa");
        let frame_va = frame_span.va;

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
        // 类别 = Task：Arc<TaskIdent>/Arc<Task> 属任务生命周期——关机 TASK_BLOCKS
        // 归零（①）。Arc 数据指针 ≠ 分配基址，装饰器无法覆盖——经标注块分配器
        // （tagged_alloc）在分配器侧标注；Arc::new_in 产 Arc<T, &'static dyn
        // Allocator>，经 into_raw_with_allocator/from_raw 转回默认分配器型
        // Arc<T>（同布局；释放路径按地址路由 + ledger 类别记账，不依赖分配器
        // 类型——见 fence::on_free）。
        let alloc = crate::memory::allocator::fence::tagged_alloc(
            crate::memory::allocator::fence::Class::Task,
        );
        let ident: Arc<TaskIdent> = unsafe {
            let (ptr, _alloc) = Arc::into_raw_with_allocator(Arc::new_in(
                TaskIdent {
                    id,
                    name: self.name,
                    team: self.team.clone(),
                    stack: stack_span,
                    frame: frame_span,
                },
                alloc,
            ));
            Arc::from_raw(ptr)
        };
        let task: Arc<Task> = unsafe {
            let (ptr, _alloc) = Arc::into_raw_with_allocator(Arc::new_in(
                Task {
                    ident,
                    state: TaskState::Starved,
                    pies: SpinLock::new(Vec::new()),
                },
                alloc,
            ));
            Arc::from_raw(ptr)
        };
        scheduler::task::push(task);
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
    // 外层 Box 以默认分配器型（Global）重建（closure 侧为 &'static dyn Allocator
    // ——同布局；释放经地址路由 + ledger 类别记账）。
    let holder: Box<Box<dyn FnOnce(), &'static dyn Allocator>> =
        unsafe { Box::from_raw(arg as *mut Box<dyn FnOnce(), &'static dyn Allocator>) };
    holder();
    scheduler::ktask::reap()
}
