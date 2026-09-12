// 本机调度器（core::hart）— per-hart 调度对象的容器与槽位操作：纯功能，无适配代码。
//
// 时间片记账：新选中任务获得满额 TIME_SLICE 预算；`advance` 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出走 `starve`：无视剩余
// 预算立即轮转——抢占与让出各自独立。
//
// 结构：Scheduler = inner(SpinLock) + badge(身份槽，无锁) + starved_len(AtomicUsize
// 锁外镜像)。身份槽的载荷类型与计数协议归 [`Badge`]：写点只有
// `Badge::{seat,shed,clear}` 三个方法（本文件的装槽 / 让位 / 降级分别调它们）。
//
// 就绪队列的改动**只有四个入口**：`starved_push` / `starved_pop` / `starved_remove` /
// `starved_clear`——计数镜像在方法体内与队列操作同一处派生（`recount`），
// `inner.starved` 对本核心之外私有、另留 `starved_is_empty` 一个持锁读法（旧版是
// 6 处手工 set_len，`rip` 的 clear 漏过一次）。steal 锁外先读 `backlog` 跳过空队列
// （不做 RMW），再 try_lock。
//
// 状态互斥：无原子字段。所有状态变更都经 Task::exclusive（唯一 Arc 所有权
// + &mut，Arc::get_mut 的 weak≥1 变体）——锁内 take/pull 出任务 → 取 &mut；
// 锁 + 所有权保证互斥，编译器强制。
//
// 锁纪律：inner = Level::Scheduler(1)，每核一把；名册 = Level::L3(**4**——3 是删掉的
// 旧槽位，名字里的 3 不是数值，见 `lock/depend.rs`）。Team.tasks(L3=4) 与
// Space.inner(Space=2) 禁止嵌套——锁内只做纯 Vec 操作，绝不调 space 方法。task
// "离开 running" 的过渡（park / wait / reap）借 `swap` 跨边界原语交给
// messenger 处理，本核只负责 settled 槽位（Live=next 或 Last）；唤醒（redeem / wipe）
// 也在 messenger。
//
// 装槽（seat）：唯一装 running 的方法，自取锁，空槽由 Option::replace 返回
// 旧值断言（绝不覆盖在跑任务），装槽顺带写身份槽（[`Badge::seat`]）。
//
// 可见性：`pub(super)` = 供本核心其余文件借用；`pub(in super::super)` = 供
// scheduler 文件夹内两个入口面（boot / trap）借用；`pub(crate)` = 供本调度器之外
// 消费（`swap` / `starve` / `push` / `running_task`）。

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};
use crate::memory::manager::addr::PhysAddr;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::unit::task::{Task, TaskIdent, TaskState};

use super::ident::Badge;

/// 新选中任务的满额时间片（量子数）。耗尽才轮转；定时器仍每量子打断，
/// 只是任务不再每量子切走。park 的 ticks 语义不受影响。
const TIME_SLICE: u32 = 8;

/// 每核调度器：真实数据在锁内，锁外只有身份槽与就绪队列长度镜像。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub(crate) struct Scheduler {
    /// 所属 hart（决定 trap 栈顶）。
    pub(super) hart: usize,
    /// 锁内：running + starved（本核调度决策的原子单位）。
    pub(super) inner: SpinLock<SchedulerInner>,
    /// 锁外：本核身份槽（[`super::ident::ident`] 的事实源）。
    pub(super) badge: Badge,
    /// 锁外：starved 长度镜像（steal 预检；与 inner 同结构体共生，不会分家）。
    /// **派生点唯一**：`starved_push` / `starved_pop` / `starved_remove` /
    /// `starved_clear` 四个方法体内的 `recount`。
    starved_len: AtomicUsize,
    /// 锁外：steal 起点游标（每次 steal 调用 fetch_add(1) % hart_count 拿起点）。
    /// 多核同时醒来时用本地游标派生不同起点——避免全从 hart 0 起步造成的 cache
    /// 热点（多 hart 同时对同一目标的 L1 锁 RMW → cache line 乒乓 = 雷鸣群）。
    /// per-hart 独立，每核 fetch_add 是 Relaxed 无需同步。
    pub(super) steal_cursor: AtomicUsize,
}

/// 锁内核心：running（运行中，不在队列）+ starved（就绪队列，FIFO）。
pub(super) struct SchedulerInner {
    pub(super) running: Option<Arc<Task>>,
    /// 就绪队列。**改动只经 [`Scheduler`] 的四个 `starved_*` 方法**（计数镜像在同一处
    /// 派生），故对本核心之外私有；跨文件只留 `starved_is_empty` 一个持锁读法。
    starved: VecDeque<Arc<Task>>,
}

impl SchedulerInner {
    /// 本核就绪队列是否空（持锁读；轮转 / 唯一任务判断用）。
    fn starved_is_empty(&self) -> bool {
        self.starved.is_empty()
    }
}

impl Scheduler {
    /// 构造（boot 适配面按实际核数逐 hart 建）。
    pub(in super::super) fn new(hart: usize) -> Scheduler {
        Scheduler {
            hart,
            inner: SpinLock::new_level(
                Level::Scheduler,
                SchedulerInner {
                    running: None,
                    starved: VecDeque::new(),
                },
            ),
            badge: Badge::new(),
            starved_len: AtomicUsize::new(0),
            steal_cursor: AtomicUsize::new(0),
        }
    }

    /// 锁外读：就绪队列长度（steal 预检；Relaxed 提示，旧读最多少偷一次）。
    pub(super) fn backlog(&self) -> usize {
        self.starved_len.load(Ordering::Relaxed)
    }

    /// 从唯一事实来源（starved.len()）重派生计数——须在持 inner 锁时调用。
    fn recount(&self, inner: &SchedulerInner) {
        self.starved_len
            .store(inner.starved.len(), Ordering::Relaxed);
    }

    // ── 就绪队列的四个改点：镜像在方法体内派生，队列与计数不可能分家 ──

    /// 队尾入队 + 派生计数。
    fn starved_push(&self, i: &mut SchedulerInner, task: Arc<Task>) {
        i.starved.push_back(task);
        self.recount(i);
    }

    /// 为本核就绪队列**预留** `slot` 格（`table::try_reserve_starved` 的实体）。
    ///
    /// # Errors
    ///
    /// 队列无法扩容（内存耗尽）→ `Err(())`。调用点在放行**之前**：那时失败还能
    /// 干净退回，而不是让 `push_back` 在内核里 panic 掉整机。
    pub(super) fn try_reserve_starved(&self, slot: usize) -> Result<(), ()> {
        self.inner.lock().starved.try_reserve(slot).map_err(|_| ())
    }

    /// 队首出队 + 派生计数；空队列 → None。
    fn starved_pop(&self, i: &mut SchedulerInner) -> Option<Arc<Task>> {
        let t = i.starved.pop_front();
        self.recount(i);
        t
    }

    /// 摘除指定任务 + 派生计数（kill 的 Starved 分支）。返回是否摘到——队列私有，
    /// 所以「找 + 摘」一起留在本文件，调用方（全机扫描）不必看队列。
    pub(super) fn starved_remove(&self, i: &mut SchedulerInner, target: &Arc<Task>) -> bool {
        let Some(pos) = i.starved.iter().position(|t| Arc::ptr_eq(t, target)) else {
            return false;
        };
        i.starved.remove(pos);
        self.recount(i);
        true
    }

    /// 清空 + 派生计数（关机）。
    pub(super) fn starved_clear(&self, i: &mut SchedulerInner) {
        i.starved.clear();
        self.recount(i);
    }

    /// 队尾入队（`launch` / 轮转 / 唤醒共用）：push + 派生计数。
    /// 只收 Starved 任务——容器 ⇔ 状态由断言强制。
    pub(crate) fn push(&self, mut task: Arc<Task>) {
        debug_assert_eq!(
            Task::exclusive(&mut task).state(),
            TaskState::Starved,
            "starved 容器只收 Starved 任务"
        );
        let mut i = self.inner.lock();
        self.starved_push(&mut i, task);
    }

    /// 队首出队（取活 / reap / park 共用）：派生计数；空队列返回 None。
    pub(super) fn pull(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.lock();
        self.starved_pop(&mut i)
    }

    /// steal 用：非阻塞取队首（锁外预检后调用）。None = 队列空或锁忙。
    pub(super) fn try_pull(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.try_lock()?;
        self.starved_pop(&mut i)
    }

    /// 任务即将在本 hart 上运行：置 Running + 满额预算 + 写 kernel_sp（本 hart
    /// trap 栈顶——steal 迁移正确性的关键）+ 武装定时器。
    fn prepare(&self, task: &mut Arc<Task>) {
        let t = Task::exclusive(task);
        t.transform(TaskState::Running {
            ticks_left: TIME_SLICE,
        });
        // SAFETY: 帧 PA 恒等映射可写；帧属 task 独占（running 或刚从 starved 摘出）。
        unsafe {
            let frame = &mut *(frame_pa(&t.ident).as_usize() as *mut TrapContext);
            frame.kernel_sp = trap_stack_edge(self.hart);
            // **核空间上下文**上台即写 tp = 本 hart PerHart 指针：内核态恒以 tp 为
            // per-hart 锚（`__core_trap` 用 `0x08(tp)` 定位 hart 帧）。判据与陷阱
            // 入口、`__restore` 的 sscratch 复原则同一条轴——`Asid::is_kernel()`，
            // **不按 S/U 分**：域任务的陷阱走 `__task_trap`，tp 由入口按 sp 反解
            // 重建，不需要这条约定，故域任务的 tp 一律留给它自己——S 态域与 U 态域
            // 同等待遇（TLS 因此对两者一视同仁）。
            if t.ident.team.space.asid().is_kernel() {
                frame
                    .gpr
                    .set_x(Gprs::TP, crate::machine::per_hart_ptr(self.hart));
            }
        }
        timer::beat(clock::duration_to_ticks(Duration::from_millis(100)));
    }

    /// 装槽：把 Starved 任务装为本 hart 的 running（自取锁）并记身份槽。空槽
    /// 由 `Option::replace` 返回旧值断言（绝不覆盖在跑任务）。
    ///
    /// 安全前提：调用方先放锁再调本方法——放锁窗口内任务已出队且唯一持有
    /// （strong == 1），无并发别名。
    pub(super) fn seat(&self, mut task: Arc<Task>) -> usize {
        let mut i = self.inner.lock();
        self.prepare(&mut task);
        let pa = frame_pa(&task.ident).as_usize();
        // 记身份（写点唯一）：载荷类型标签与计数协议都在 Badge 内（含旧载荷回收）。
        self.badge.seat(&task.ident);
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Running { .. }
            ),
            "running 容器只装 Running 任务"
        );
        // 装槽：replace 完成实际装槽（副作用不得藏在 debug_assert 内——release
        // 下断言被编译掉，装槽即失效 → running 恒空）。再断言旧槽必空（seat
        // 唯一装槽点）。
        let prev = i.running.replace(task);
        debug_assert!(prev.is_none(), "装槽前 running 必须为空");
        // 装槽完成 → 载荷为 TaskIdent（Live：trap 可信）。身份槽的 AcqRel swap
        // 已发布 prepare 写出的帧/任务状态；[`super::ident::ident`] 的 Acquire 配对。
        pa
    }

    // 注：关机清理（原 clear_slot）与降级（原 shed）不在这里——身份槽的两态与计数
    // 协议整个归 [`Badge`]：`Badge::shed`（reap / park 无后继：TaskIdent → LastIdent）、
    // `Badge::clear`（关机基线审计前清空槽载荷，否则每 hart 末次 LastIdent 计入块差集
    // 误报泄漏：已实证 4 hart = 4 个 48B 假泄漏）。本核只负责 running 槽。

    /// 跨边界原语（messenger 三种过渡共用）：取走 running + 装下一 starved 或
    /// 降级身份槽。返回 (取走的 Arc<Task>, Optional 下一帧 PA)。
    ///
    /// 锁纪律：内锁取 running / 弹 starved 后立即放；seat 重新取内锁。
    /// messenger 在两次取锁之间做自己的簿记（sites / holders / husks
    /// 各自 L3 锁，绝不持 L3 取 L1）。
    pub(crate) fn swap(&self) -> (Arc<Task>, Option<usize>) {
        let mut i = self.inner.lock();
        let task = i.running.take().expect("no running task");
        let ident = task.ident.clone();
        let next = self.starved_pop(&mut i);
        drop(i);
        let next_pa = if let Some(next) = next {
            let pa = frame_pa(&next.ident).as_usize();
            self.seat(next);
            Some(pa)
        } else {
            self.badge.shed(&ident);
            None
        };
        (task, next_pa)
    }

    /// 当前 running 任务的 Arc 克隆（mail 模块持有 mail 接入点用——Arc 共享
    /// 借出，不取走 running 槽）。**调用方负责 push 到 task.mail；不允许持锁
    /// 跨调用**（inner L1 与 mail L3 同层嵌套会 lockdep 违规——先克隆 Arc 再
    /// 放 inner 锁，再去取 mail 锁）。
    pub(crate) fn running_task(&self) -> Option<Arc<Task>> {
        let i = self.inner.lock();
        i.running.as_ref().map(Arc::clone)
    }

    /// 轮转尾部（持锁、starved 非空）：Running → Starved 入队尾，队首上台。
    /// 调用方负责空队列判断（空 → 唯一任务续跑，不走本方法）。
    fn rotate(&self, i: &mut SchedulerInner, mut cur: Arc<Task>) -> Arc<Task> {
        Task::exclusive(&mut cur).transform(TaskState::Starved);
        self.starved_push(i, cur);
        self.starved_pop(i).expect("non-empty")
    }

    /// 主动让出：无视剩余预算立即轮转（Running → Starved）。
    ///
    /// `pub(crate)`：唯一调用方是 envcall 的 `RoomCall::Starve`（任务面文件删除后
    /// 直呼本方法，不再经 `utask::starve` 转发）。
    pub(crate) fn starve(&self) -> usize {
        let mut i = self.inner.lock();
        let Some(cur) = i.running.take() else {
            panic!("starve with no running task on hart {}", self.hart);
        };
        if i.starved_is_empty() {
            // 本 hart 唯一任务：无需轮转，继续运行
            let pa = frame_pa(&cur.ident).as_usize();
            i.running = Some(cur);
            return pa;
        }
        let prev_tid = cur.ident.id;
        let next = self.rotate(&mut i, cur);
        drop(i);
        let pa = self.seat(next);
        trace::note(EventKind::Room(RoomEvent::Starve { tid: prev_tid }));
        pa
    }

    /// 本核时间片的唯一决策点：续跑（只减计数不重排）或轮转（Running → Starved，
    /// 队首上台）；`running` 槽为空 → `None`（取活接手）。
    ///
    /// 续跑两分支合并（语义等价：先判后 dec——先判是否续跑，再在分支内递减；
    /// 若先 dec 再判，pre=2 且他队非空会提前轮转一格）。两情形均不切走、
    /// 不进 starved；预算恒 ≥ 1 不落盘（唯一任务分支不减预算）。
    pub(in super::super) fn advance(&self) -> Option<usize> {
        let mut i = self.inner.lock();
        let mut cur = i.running.take()?;
        // 持有者读：running 槽刚被本核摘出，唯一强持有 ⇒ 经 exclusive 拿 &mut。
        let ticks_left = match Task::exclusive(&mut cur).state() {
            TaskState::Running { ticks_left } => ticks_left,
            _ => unreachable!("running 容器里不是 Running 任务"),
        };
        if ticks_left > 1 || i.starved_is_empty() {
            if ticks_left > 1 {
                Task::exclusive(&mut cur).dec_ticks_left();
            }
            let pa = frame_pa(&cur.ident).as_usize();
            i.running = Some(cur);
            return Some(pa);
        }
        let prev_tid = cur.ident.id;
        let next = self.rotate(&mut i, cur);
        let next_tid = next.ident.id;
        drop(i);
        // Switch 事件落在身份槽更新（seat）**之后**：窗口内崩溃不再把已下台
        // 的 prev 报成当前任务（轮转窗口 issue）。
        let pa = self.seat(next);
        trace::note(EventKind::Room(RoomEvent::Switch { prev_tid, next_tid }));
        Some(pa)
    }

    // 注：park / wait / reap 三个 Scheduler 方法已移至 [`crate::work::room::messenger`]，
    // 任务"离开 running 槽"的所有过渡归 messenger 管理——它们借 Scheduler::swap
    // 跨边界原语完成槽位 settled，再在 messenger 域内做 sites / husks 簿记。
}

/// 帧物理地址：`Span.pa` 的真相在 `space::Span`（trap 帧恒 Some；栈/懒区恒 None），
/// 本核心只在这里说一次这话。
pub(super) fn frame_pa(ident: &TaskIdent) -> PhysAddr {
    ident.frame.pa.expect("frame span has pa")
}
