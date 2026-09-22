// 本机调度器（core::hart）— per-hart 调度对象的容器与槽位操作：纯功能，无适配代码。
//
// 时间片记账：新选中任务获得满额 QUANTUM_TICKS 预算；`advance` 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出走 `starve`：无视剩余
// 预算立即轮转——抢占与让出各自独立。
//
// 结构：Scheduler = inner(SpinLock) + badge(身份槽，无锁)。身份槽的载荷类型与计数
// 协议归 [`Badge`]：写点只有 `Badge::{seat,shed}` 两个方法（本文件的装槽 / 降级
// 分别调它们）。
//
// 就绪队列的改动**只有四个入口**：`starved_push` / `starved_pop` / `starved_remove` /
// `starved_clear`；`inner.starved` 对本核心之外私有，跨文件只留 `starved_is_empty`
// 一个持锁读法。**照实记**：旧版这里还有一面 `starved_len` 锁外镜像 + `backlog()` 预检
// （给 `steal` 跳过空队列用），`steal` 已删，镜像随之退休（链长不再有第二份事实）。
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

use alloc::sync::Arc;

use crate::lock::{Level, SpinLock};
use crate::memory::manager::addr::PhysAddr;
use crate::runtime::chrono::timer;
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::unit::task::{Task, TaskIdent, TaskState};

use super::ident::Badge;

/// 新选中任务的满额预算，**单位是"拍"（tick）不是毫秒**——名字里带单位就是为了一眼看住
/// 这一点。耗尽才轮转；定时器仍每拍打断，只是任务不再每拍切走。park 的 ticks 语义不受影响。
///
/// # 照实记：所以"量子"不是时间，而且它被**别人的到点登记**牵着走
///
/// 一拍的实测长度 = `min(最近活到点, chrono::timer::BLIND_MS)`（武装点见
/// `runtime::switcher::trap` 与 `fetch::wait`），于是**有效量子 ∈ (0, 8 × 100 ms]**：
///
/// - 有域登记了 1 ms 的到点 ⇒ 全场拍长变 1 ms ⇒ **所有**任务的量子缩到 8 ms；
/// - 一个到点都没有 ⇒ 拍长吃满 `BLIND_MS = 100 ms` ⇒ 一个任务可连跑 **800 ms** 才轮转。
///
/// 这条全局耦合是**今天的事实**（照实记），不是设计承诺。若以后要一个**可承诺的**量子
/// （按时间而不是按拍），那是结构门：`TaskState::Running { ticks_left }` 的载荷要改成
/// "上台时刻 + 预算"，`advance` 按经过的时间判——那时再开。
///
/// 另注：量子只在**会吃定时器陷阱**的上下文里生效——S 态域任务空转不吃陷阱
/// （实测：单核 6 s 纯空转 `traps` 不涨）⇒ 量子对它等于不存在。别以为它管一切任务。
const QUANTUM_TICKS: u32 = 8;

/// 每核调度器：真实数据在锁内，锁外只有身份槽与就绪队列长度镜像。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub(crate) struct Scheduler {
    /// 所属 hart（决定 trap 栈顶）。
    pub(super) hart: crate::hart::HartId,
    /// 锁内：running + starved（本核调度决策的原子单位）。
    pub(super) inner: SpinLock<SchedulerInner>,
    /// 锁外：本核身份槽（[`super::ident::ident`] 的事实源）。
    pub(super) badge: Badge,
}

/// 锁内核心：running（运行中，不在队列）+ starved（就绪队列，FIFO）。
pub(super) struct SchedulerInner {
    pub(super) running: Option<Arc<Task>>,
    /// 就绪队列 = **一条穿在任务 `Starved` 载荷里的链**，这里只存两头。
    ///
    /// 改动只经 [`Scheduler`] 的四个 `starved_*` 方法，故对本核心之外私有；跨文件只留
    /// `starved_is_empty` 一个持锁读法。链的下一环在任务自己身上（`Task::starved_next`），
    /// 互斥由本 `inner` 锁 + 容器唯一性保证。
    ///
    /// 为什么不是 `VecDeque`：唤醒路径（`rise`：`wake`/`wipe`/`redeem`）与轮转路径
    /// **没有失败域**，任何"要么扩容要么 halt"的容器在这两条路上都是地雷；而队列的
    /// 容量需求是"并发占用"，跟"一生一次"的预留对不上（`VecDeque::try_reserve(1)`
    /// 不累加：备下的那一格会被同键的另一个等待者先占走）。
    head: Option<Arc<Task>>,
    /// 链尾（多持一个强引用，等价 Linux `rb_leftmost` 那种缓存；链的所有权仍在节点间）。
    tail: Option<Arc<Task>>,
}

impl SchedulerInner {
    /// 本核就绪队列是否空（持锁读；轮转 / 唯一任务判断用）。
    fn starved_is_empty(&self) -> bool {
        self.head.is_none()
    }
}

impl Scheduler {
    /// 构造（boot 适配面按实际核数逐 hart 建）。
    pub(in super::super) fn new(hart: crate::hart::HartId) -> Scheduler {
        Scheduler {
            hart,
            inner: SpinLock::new_level(
                Level::Scheduler,
                SchedulerInner {
                    running: None,
                    head: None,
                    tail: None,
                },
            ),
            badge: Badge::new(),
        }
    }

    // ── 就绪队列的四个改点：镜像在方法体内派生，队列与计数不可能分家 ──

    /// 队尾入队（**纯指针写，零分配**）+ 计数 ±1。
    ///
    /// 前置（断言兜底）：入队者状态为 `Starved { next: None }`——容器只收 Starved
    /// 任务，且它不得还挂在别的链上（否则就是一个任务两条链）。
    fn starved_push(&self, i: &mut SchedulerInner, mut task: Arc<Task>) {
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Starved { next: None }
            ),
            "starved 容器只收 Starved 任务，且入队前不得挂在链上"
        );
        match i.tail.take() {
            // 空队列：新节点即链头。
            None => i.head = Some(task.clone()),
            // 非空：接到原链尾的载荷上，再把链尾前移。
            Some(mut last) => *Task::starved_next(&mut last) = Some(task.clone()),
        }
        i.tail = Some(task);
    }

    /// 队首出队（**摘链 + 清空离开者的 `next`**）+ 计数 −1；空队列 → None。
    ///
    /// 清空那一步是硬要求：不清就等于"被取走的任务还持有它原来的后继"，同一段链
    /// 会有两个所有者（偷窃路径正靠它保证不把任务留在两条队列里）。
    fn starved_pop(&self, i: &mut SchedulerInner) -> Option<Arc<Task>> {
        let mut head = i.head.take()?;
        i.head = Task::starved_next(&mut head).take();
        if i.head.is_none() {
            i.tail = None;
        }
        Some(head)
    }

    /// 摘除指定任务 + 派生计数（kill 的 Starved 分支）。返回是否摘到——队列私有，
    /// 所以「找 + 摘」一起留在本文件，调用方（全机扫描）不必看队列。
    ///
    /// **链尾**跟着改：摘掉的若是最后一环（`next` 为空），新链尾就是它的前驱。只在新
    /// 链为空时清 `tail` 不够——摘掉"非头的尾"会留下一个指着链外的 `tail`，下一次
    /// `starved_push` 就把新任务接到链外节点上（链头到不了它 = 任务静静丢失）。
    pub(super) fn starved_remove(&self, i: &mut SchedulerInner, target: &Arc<Task>) -> bool {
        // 走链：`prev` 是"摘除点"的持有者（None = 摘链头）。逐节 clone 只为比较身份，
        // 不动链；命中时把后继接到前驱的载荷上，并清空离开者。
        let mut prev: Option<Arc<Task>> = None;
        let mut cur = i.head.clone();
        while let Some(mut node) = cur {
            if Arc::ptr_eq(&node, target) {
                let next = Task::starved_next(&mut node).take();
                let was_tail = next.is_none();
                match &mut prev {
                    Some(p) => *Task::starved_next(p) = next,
                    None => i.head = next,
                }
                if was_tail {
                    i.tail = prev;
                }
                return true;
            }
            prev = Some(node.clone());
            cur = Task::starved_next(&mut node).clone();
        }
        false
    }

    /// 清空 + 派生计数（关机）。
    pub(super) fn starved_clear(&self, i: &mut SchedulerInner) {
        // 逐节摘（每节先 `take()` 再 drop）：整条链一次性 drop 会把长链压进调用栈。
        while self.starved_pop(i).is_some() {}
    }

    /// 队尾入队（**唯一调用方 = `table::kick`**）：push + 派生计数。
    /// 只收 Starved 任务——容器 ⇔ 状态由断言强制。
    ///
    /// 「唯一」是甲案的不变量（入队者 = 被叫醒者）在代码上的落点：轮转与 `swap` 取下一枚
    /// 走本文件内的 `starved_push`（它们不是"产生活"，是本核自产自销），跨核投活一律经
    /// [`super::table::kick`] ——那里先入队、再按 `conductor::waiting` 决定要不要 IPI。
    /// 别人再开一口 `push` 就等于绕过那记 IPI：活进队列而没人被告知。
    pub(crate) fn push(&self, mut task: Arc<Task>) {
        debug_assert!(
            matches!(
                Task::exclusive(&mut task).state(),
                TaskState::Starved { .. }
            ),
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

    /// 任务即将在本 hart 上运行：置 Running + 满额预算 + 写 kernel_sp（本 hart
    /// trap 栈顶）+ 武装定时器。
    fn prepare(&self, task: &mut Arc<Task>) {
        let t = Task::exclusive(task);
        t.transform(TaskState::Running {
            ticks_left: QUANTUM_TICKS,
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
                    .set_x(Gprs::TP, crate::hart::per_hart_ptr(self.hart));
            }
        }
        // 武装点 = min(本核上限, 最近活到点)——上限即**失明上限**（旧写法把它叫"抢占量子"，
        // 正是量子与失明上限混为一谈的由来，见 `QUANTUM_TICKS` 的照实记）。
        timer::beat_until(timer::blind_ceiling());
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

    // 注：降级（原 shed）不在这里——身份槽的两态与计数协议整个归 [`Badge`]：
    // `Badge::shed`（reap / park 无后继：TaskIdent → 末次两枚号）把末次身份压进那
    // 一格字，不分配、无计数可还，故没有对应的清槽原语。本核只负责 running 槽。

    /// 跨边界原语（messenger 三种过渡共用）：取走 running + 装下一 starved 或
    /// 降级身份槽。返回 (取走的 Arc<Task>, Optional 下一帧 PA)。
    ///
    /// 锁纪律：内锁取 running / 弹 starved 后立即放；seat 重新取内锁。
    /// messenger 在两次取锁之间做自己的簿记（sites / holders / husks
    /// 各自 L3 锁，绝不持 L3 取 L1）。
    pub(crate) fn swap(&self) -> (Arc<Task>, Option<usize>) {
        let mut i = self.inner.lock();
        let task = i.running.take().expect("no running task");
        let next = self.starved_pop(&mut i);
        drop(i);
        let next_pa = if let Some(next) = next {
            let pa = frame_pa(&next.ident).as_usize();
            self.seat(next);
            Some(pa)
        } else {
            self.badge.shed(&task.ident);
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
        Task::exclusive(&mut cur).transform(TaskState::Starved { next: None });
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
        let prev_tid = cur.ident.id.get();
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
            TaskState::Running { ticks_left } => *ticks_left,
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
        let prev_tid = cur.ident.id.get();
        let next = self.rotate(&mut i, cur);
        let next_tid = next.ident.id.get();
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
