// 线程（可调度单元）— 类型 + 构造。
//
// Task = 可调度单元：共享所属 Team 的地址空间，持有自己的 trap 帧。
// TaskBuilder 在团队容器内生成任务：栈 + trap 帧 + 填帧 + 入队。
//
// **两段式构造**（D3=B 的顺序要求）：`hold` 产 `Held`（未放行、已入簿记与计数），
// `spawn` = `hold` + 立即放行。跨域产线程必须走 `hold`，父方 `Accord` 之后再
// `Hatch`——新线程的权限表起步为空，「先授权、后运行」是安全的一侧。

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
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::TrapContext;
use crate::work::room::conductor;
use crate::work::unit::gate::{AnyPie, GateError};

use crate::work::unit::space::window::{FrameWindow, StackWindow};
use crate::work::unit::team::kernel;

use super::team::Team;
use crate::work::room::messenger::{Ticket, WakeKey};
use crate::work::room::scheduler;
use env::TeamId;

/// 全局任务号（跨 hart 唯一）。自 1 起：0 保留作「无任务」哨兵——`SelfId`/
/// `sire()` 等以 0 表「无上下文 / 无父」，真实 task id 恒 ≥ 1，哨兵无歧义。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// 启动参数上限（字）。栈顶 args 区 ≤ 512 B；超出由适配层拒（`-1 Denied`）。
pub(crate) const MAX_ARGS: usize = 64;

/// 该 task id 是否**已被分配过**。
///
/// 注册表只存 `Weak` 且从不清理，故「已回收」与「从未存在」都升级失败——
/// `Join` 用本判据区分：已分配 ⇒ 已回收（当场 `true`）；未分配 ⇒ 非法 id（Denied）。
pub(crate) fn allocated(id: usize) -> bool {
    id < NEXT_ID.load(Ordering::Relaxed)
}

/// 任务状态：任务现在在哪 +（Running/Blocked 时）该状态特有的数据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// 正在执行（恒为某 hart 的 running，不在任何队列）：预算随 run 递减。
    /// 不变量：预算恒 ≥ 1（耗尽即转 Starved，不落盘 Running{0}）。
    Running { ticks_left: u32 },
    /// 已阻塞（在站点队列里；不在任何就绪队列，不可被 steal）：等待点在载荷。
    ///
    /// **等待点 = 键 + 票**：键指向站点（唤醒侧按它找人），票指向到点登记（到期侧
    /// 凭它认领）。两者都在这里，故「谁在等、等什么、等到何时」不散在全局表里。
    Blocked { key: WakeKey, ticket: Ticket },
    /// 已饥饿（预算耗尽，在 starved 容器等补给；被选中时重置满额预算）。
    Starved,
    /// **未放行**（在 `Team.held` 里；不在任何队列，不可被 steal）：`Spawn` 的初始态，
    /// 只能经 `Hatch` 转 Starved（或随父域被扑杀转 `Doomed`）。
    Held,
    /// **已停摆**（摘出了全部调度/等待容器，退出钩子未跑；不在任何调度队列）。
    ///
    /// 只由 `messenger::suspend` 置位，是「判死」与「收尾」之间的过渡态：扑杀整棵
    /// 血缘子树时**先让全部受害者停摆、再逐个跑钩子**——钩子会摘门闩、唤醒等待者，
    /// 若此时还有受害者能被唤醒后运行，它就会在注定要死的状态下看到已死资源。
    Doomed,
    /// 已收割（躯壳，在 reaped 容器等延迟回收；不在任何调度队列，任何核可回收）。
    ///
    /// 不变量：**退出钩子已跑完**——唯一置位路径是 `messenger::die`（钩子 → 本态 →
    /// 入躯壳队列），故「`state == Reaped`」精确表示**收尾已完成**，`Join` 的判据
    /// 因此不含竞态。延迟的是**回收**（栈/trap 帧/团队空间），不是收尾。
    Reaped,
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
    /// 我生的子域（强持有，血缘清单）。三合一角色：撑命（无线程子域靠它活）、
    /// `spawn` 授权凭证（能在我 heir 里查到 = 我是 sire）、`doom` 级联遍历源。
    /// 锁级 = L3（与 pies 同级）。强持有与 `Team.sire`（弱）配对断环。
    pub(crate) heir: SpinLock<Vec<Arc<Team>>>,
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
    ///   Held → Starved（放行）
    ///   Starved → Running（调度器选上 / steal 迁移后运行）
    ///   Running → Starved（预算耗尽轮转 / 主动让出）
    ///   Running → Blocked(原因)（阻塞：如睡眠）
    ///   Blocked(_) → Starved（唤醒：回到就绪容器）
    ///   {Held, Starved, Blocked, Running} → Doomed（停摆：判死，钩子未跑）
    ///   Doomed → Reaped（收尾：退出钩子已跑完，入躯壳队列）
    pub(crate) fn transform(&mut self, next: TaskState) {
        let legal = matches!(
            (self.state, next),
            (TaskState::Held, TaskState::Starved)
                | (TaskState::Starved, TaskState::Running { .. })
                | (TaskState::Running { .. }, TaskState::Starved)
                | (TaskState::Running { .. }, TaskState::Blocked { .. })
                | (TaskState::Blocked { .. }, TaskState::Starved)
                | (TaskState::Held, TaskState::Doomed)
                | (TaskState::Starved, TaskState::Doomed)
                | (TaskState::Blocked { .. }, TaskState::Doomed)
                | (TaskState::Running { .. }, TaskState::Doomed)
                | (TaskState::Doomed, TaskState::Reaped)
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
    /// 调用方义务：任务**至少**被一个容器强持有（running / starved / blocked /
    /// reaped / held 之一）→ strong ≥ 1。envcall 路径（vest / 未来远程操作）可短暂持额
    /// 外强引用，**但不解引用 Task 字段**——只 `Arc::ptr_eq` / 借用 pies 锁 /
    /// drop；唯一改 Task 字段的路径是 `transform`，由本函数串起。
    /// 互斥仍由调度器锁 + 容器唯一性保证；debug 断言只兜底"无主"漏 ref。
    pub(crate) fn exclusive(t: &mut Arc<Self>) -> &mut Task {
        #[cfg(debug_assertions)]
        assert!(
            Arc::strong_count(t) >= 1,
            "task #{} '{}': no holders (strong_count == 0)",
            t.ident.id,
            t.ident.name
        );
        // SAFETY: 至少一个容器持强引用 ⇒ transform 路径独占（其他 envcall 临时
        // 持有者不触字段）；Team 簿记弱引用不读字段。等价 Arc::get_mut（其要求
        // weak == 0），放宽 strong_count 后允许多个容器 + 临时强引用并存。
        unsafe { &mut *Arc::as_ptr(t).cast_mut() }
    }

    /// 放行（`Held → Starved` 入队）。**不做授权**——授权在适配层（envcall）。
    ///
    /// 前置：目标仍在所属 `Team.held` 里且状态为 `Held`；否则 `Denied`
    /// （放行只发生一次，不静默）。
    pub(crate) fn release(task: &Arc<Task>) -> Result<(), GateError> {
        let team = task.ident.team.clone();
        match team.take_held() {
            Some(held) if Arc::ptr_eq(&held, task) => {}
            other => {
                // 不是引导线程（或已被摘出）：放回去，报 Denied。
                if let Some(t) = other {
                    team.hold(&t);
                }
                return Err(GateError::Denied);
            }
        }
        let mut t = task.clone();
        Task::exclusive(&mut t).transform(TaskState::Starved);
        scheduler::task::push(t);
        Ok(())
    }

    /// 记我生的子域（强持有）。由 `TeamBuilder::spawn` 调用——**唯一入口**
    /// （K1 血缘闭合；`spawn` 之外不得再调）。
    pub(crate) fn adopt(&self, child: Arc<Team>) {
        self.heir.lock().push(child);
    }

    /// 快照我的全部子域（doom 级联遍历用：快照后放锁，锁外逐条处理）。
    pub(crate) fn heirs(&self) -> Vec<Arc<Team>> {
        self.heir.lock().clone()
    }

    /// 在我生的子域里按 id 查（`spawn` 授权：查到 = 我是 sire）。
    pub(crate) fn heir(&self, id: TeamId) -> Option<Arc<Team>> {
        self.heir.lock().iter().find(|t| t.id == id).cloned()
    }

    /// 我生的子域数量（heir 枚举 first pass）。
    pub(crate) fn heir_count(&self) -> usize {
        self.heir.lock().len()
    }

    /// 按索引取子域 TeamId（heir 枚举 second pass；越界 → None）。
    pub(crate) fn heir_at(&self, index: usize) -> Option<TeamId> {
        self.heir.lock().get(index).map(|t| t.id)
    }
}

/// 启动参数写入新任务栈顶（`at` 起 `args.len()` 个字）。
///
/// 栈体在 `StackWindow::claim` 时已逐页物化，故 `translate` 必成——不成即内核
/// 不变量破裂，直接 panic（同 `frame span has pa` 的纪律）。跨页按页写。
fn write_args(space: &crate::work::unit::space::Space, at: VirtAddr, args: &[usize]) {
    let mut done = 0usize;
    while done < args.len() {
        let va = at + done * size_of::<usize>();
        let (pa, _) = space
            .translate(va)
            .expect("stack page materialized before args write");
        let page_rest = PAGE_SIZE - (va.as_usize() % PAGE_SIZE);
        let n = core::cmp::min((args.len() - done) * size_of::<usize>(), page_rest)
            / size_of::<usize>();
        let dst = pa.as_usize() as *mut usize;
        for i in 0..n {
            // SAFETY: 帧由本空间独占持有（新任务尚未入队）；恒等映射下 PA 可写。
            unsafe { core::ptr::write_volatile(dst.add(i), args[done + i]) };
        }
        done += n;
    }
}

/// 任务构建器：在团队容器内生成线程（栈 + trap 帧 + 填帧 + 入队）。
///
/// 入口参数 `args` 写入新任务栈顶，寄存器约定 `a0 = args VA`、`a1 = count`。
///
/// # Errors
///
/// 栈/帧分配失败（MapError 原样传播）；失败时已分配资源随 Space drop 回滚。
pub struct TaskBuilder {
    team: Arc<Team>,
    name: &'static str,
    entry: VirtAddr,
    args: Vec<usize>,
    /// 栈体大小（页对齐；缺省 `TASK_STACK_SIZE`）。
    stack: usize,
}

impl TaskBuilder {
    /// 在指定团队内生成任务。入口默认 = **域的默认入口**（`Build` 装载所得
    /// `e_entry`）；域未设（内核团队）时退回 `IMAGE_BASE`。
    pub fn new(team: Arc<Team>) -> TaskBuilder {
        let entry = match team.default_entry() {
            0 => IMAGE_BASE,
            e => VirtAddr::from_raw(e),
        };
        TaskBuilder {
            team,
            name: "task",
            entry,
            args: Vec::new(),
            stack: TASK_STACK_SIZE,
        }
    }

    /// 线程名（默认 "task"；诊断用角色名——域名字在 `Team.name`）。
    pub fn name(mut self, name: &'static str) -> TaskBuilder {
        self.name = name;
        self
    }

    /// 启动参数（写入新任务栈顶；`a0 = args VA`、`a1 = count`）。
    pub fn args(mut self, args: Vec<usize>) -> TaskBuilder {
        debug_assert!(args.len() <= MAX_ARGS, "args 超过 MAX_ARGS");
        self.args = args;
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
    /// 目录（原唯一使用者）已移出内核、跑在 `task-dir` 域里，故本面暂无树内使用者，
    /// 保留作内核线程原语。
    #[allow(dead_code)]
    pub fn closure<F>(self, f: F) -> Result<Arc<Task>, MapError>
    where
        F: FnOnce() + Send + 'static,
    {
        debug_assert!(
            self.team.space.asid().is_kernel(),
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
        // SAFETY: 闭包在本地装箱，args[0] 传其薄指针；SPP=1 回 S 态运行于
        // `ktask_trampoline`（该 trampoline 从 `a0` 指向的 args 区读指针）。
        let entry = VirtAddr::from_raw(ktask_trampoline as *const () as usize);
        self.entry(entry).args(alloc::vec![ptr]).spawn()
    }

    /// 产**未放行**线程：栈 slot + trap 帧（入团队空间窗口簿记）→ 写 args →
    /// 填帧 → 入簿记（`Team.tasks`）+ 进 `Team.held` + 计数（PUSHED）。
    ///
    /// 计数在**产生**处而非入队处：Held 线程若被父域 `kill`，`REAPED` 与 `PUSHED`
    /// 必须仍然配平——否则 `done()` 恒假，系统永不停机。
    pub fn hold(self) -> Result<Arc<Task>, MapError> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

        // 栈：StackWindow::claim 取 slot（user 段 + guard，立即物化；U 位随空间模式）
        let stack_size = self.stack;
        let stack_span = StackWindow::claim(&self.team.space, stack_size)?;
        // 栈体基址（供填帧算 stack_top）= slot 基址 + guard
        let stack_body = stack_span.va + crate::layout::TASK_STACK_GUARD;
        let stack_body_top = stack_body.as_usize() + stack_size;

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

        // args 区：栈顶之下 count 个字；初始 sp = 16 对齐后的 args 区下界。
        let count = self.args.len();
        let args_at = stack_body_top - count * size_of::<usize>();
        if count > 0 {
            write_args(&self.team.space, VirtAddr::from_raw(args_at), &self.args);
        }
        let sp = VirtAddr::from_raw(args_at & !0xF);

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
                &*ktc,
                &self.team,
                self.entry,
                sp,
                (VirtAddr::from_raw(args_at), count),
                frame_pa,
                frame_va,
            );
        }

        // 入队收尾（**不入调度队列**——等 `Hatch`）
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
                    state: TaskState::Held,
                    pies: SpinLock::new(Vec::new()),
                    heir: SpinLock::new(Vec::new()),
                },
                alloc,
            ));
            Arc::from_raw(ptr)
        };
        scheduler::core::register_task_id(id, &task);
        // 簿记 + 未放行容器 + 产生计数（配对见函数头）
        self.team.push_task(&task);
        self.team.hold(&task);
        conductor::push();
        trace::note(EventKind::Room(RoomEvent::Spawn { tid: id }));
        Ok(task)
    }

    /// 产线程并**立即放行**（`hold` + `Hatch`）。boot 装 root 用。
    pub fn spawn(self) -> Result<Arc<Task>, MapError> {
        let task = self.hold()?;
        Task::release(&task).expect("freshly held task must release");
        Ok(task)
    }
}

/// 内核任务 trampoline：解包闭包、执行、跑完自动退出。
///
/// `a0` = args 区 VA（`TaskBuilder::args` 写入的**数组**地址）；本函数读
/// `args[0]` 得 `Box<dyn FnOnce()>` 薄指针。该函数作为内核任务的 sepc 入口，
/// SPP=1 回 S 态执行于该任务内核栈上；闭包返回后退出调度。
///
/// 必须以 `-> !` 返回：从 `_start`-式入口返回会跳 0 崩溃，退出必须显式执行。
///
/// # Safety
/// `arg` 必须是 `TaskBuilder::closure` 产出的 args 区 VA（`args[0]` 为其
/// 闭包装箱的薄指针）。
#[allow(dead_code)] // 内核线程面：暂无树内使用者（目录已移出内核）
pub(crate) extern "C" fn ktask_trampoline(arg: usize) -> ! {
    // tp = 本 hart PerHart 指针：每个内核任务上台时 Scheduler::prepare 已把 TP
    // 写入其帧（frame.gpr[TP] = per_hart_ptr(self.hart)），__restore 恢复全部 GPR
    // 时 tp 即已在位——此处不再重建。
    // SAFETY: arg 指向本任务栈上的 args 区（a0 由填帧写入）；args[0] 由 closure
    // 以 Box::into_raw(holder) 产出（薄指针），此处独占回收。
    let ptr = unsafe { core::ptr::read_volatile(arg as *const usize) };
    let holder: Box<Box<dyn FnOnce(), &'static dyn Allocator>> =
        unsafe { Box::from_raw(ptr as *mut Box<dyn FnOnce(), &'static dyn Allocator>) };
    holder();
    scheduler::ktask::reap()
}
