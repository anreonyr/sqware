// 指令调度核心（scheduler::core）— per-hart 调度：纯功能，无适配代码。
//
// 时间片记账：新选中任务获得满额 TIME_SLICE 预算；run 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出走 starve：无视剩余
// 预算立即轮转——抢占与让出各自独立。
//
// 结构：Scheduler = inner(SpinLock) + info(身份槽，无锁) + starved_len(AtomicUsize
// 锁外镜像)。info 槽 = `ident()` 的事实源：带标签指针（bit0 = 载荷
// 类型：TaskIdent 在跑 / LastIdent 末次记录），写 = 本核 seat/shed 的 swap（AcqRel），
// 读 = 本核 trap/panic——同 hart 单写单读 + 载荷不可变 ⇒ 无锁（跨核读是 UB，字段私有
// 且只经 ident() 触及）。
//
// 就绪队列的改动**只有四个入口**：`starved_push` / `starved_pop` / `starved_remove` /
// `starved_clear`——计数镜像在方法体内与队列操作同一处派生，`inner.starved` 对 core.rs
// 之外私有、另留 `starved_is_empty` 一个持锁读法（旧版是 6 处手工 set_len，`rip` 的
// clear 漏过一次）。steal 锁外先读 starved_len 跳过空队列（不做 RMW），再 try_lock。
//
// 状态互斥：无原子字段。所有状态变更都经 Task::exclusive（唯一 Arc 所有权
// + &mut，Arc::get_mut 的 weak≥1 变体）——锁内 take/pull 出任务 → 取 &mut；
// 锁 + 所有权保证互斥，编译器强制。
//
// 锁纪律：inner = Level::Scheduler(1)，每核一把；名册 = Level::L3(**4**——3 是删掉的
// 旧槽位，名字里的 3 不是数值，见 `lock/depend.rs`)。Team.tasks(L3=4) 与
// Space.inner(Space=2) 禁止嵌套——锁内只做纯 Vec 操作，绝不调 space 方法。task
// "离开 running" 的过渡（park / wait / reap）借 disown_and_install_next 跨边界原语交给
// messenger 处理，本核只负责 settled 槽位（Live=next 或 Last）；唤醒（redeem / wipe）
// 也在 messenger。
//
// 装槽（seat）：唯一装 running 的方法，自取锁，空槽由 Option::replace 返回
// 旧值断言（绝不覆盖在跑任务）。装槽写 info 身份槽（TaskIdent 载荷）；降级
// （shed：reap / park 无后继）换 LastIdent 载荷——写点唯一 pair（同标签原子）。
//
// 可见性：`pub(super)` = 供本文件夹各适配面借用的核心表面（入口面转发点）；
// `pub` = 供 scheduler 之外消费（ident —— 身份槽读取）。wait() 是 WFI 入口
// 借 messenger::redeem 处理 timer 到期；bury 不归本核管。

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use core::time::Duration;
use hashbrown::HashMap;

use riscv::register::sip;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::machine;
use crate::memory::manager::addr::PhysAddr;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::room::conductor;
use crate::work::room::messenger;

use crate::work::unit::task::{Task, TaskIdent, TaskState};

// ── 核心：常量 ──

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」。
const WFI_FAR: u64 = 1 << 60;
/// 身份槽载荷类型标签（bit0）：0 = TaskIdent（在跑任务），1 = LastIdent（末次
/// 记录）。标签与指针同一原子字——载荷类型自描述，读侧无需第二读点。
const LAST_TAG: usize = 1;

/// 新选中任务的满额时间片（量子数）。耗尽才轮转；定时器仍每量子打断，
/// 只是任务不再每量子切走。park 的 ticks 语义不受影响。
const TIME_SLICE: u32 = 8;

// ── 核心：per-hart 调度器结构与方法 ──

/// 每核调度器：真实数据在锁内，锁外只有身份槽与 starved 长度镜像。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub(crate) struct Scheduler {
    /// 所属 hart（决定 trap 栈顶）。
    hart: usize,
    /// 锁内：running + starved（本核调度决策的原子单位）。
    pub(super) inner: SpinLock<SchedulerInner>,
    /// 锁外：本核当前任务身份（`ident()` 的事实源）。**带标签指针**：bit0 = 载荷
    /// 类型标签（0 = TaskIdent / 1 = LastIdent）。原子指针 + 手工 Arc 计数：写 =
    /// 本核装槽（seat，AcqRel swap，取走旧指针按标签收回归还其计数）或降级
    /// （shed，TaskIdent → LastIdent），读 = 本核 trap/panic 路径 load（Acquire）
    /// + increment_strong_count——载荷不可变 ⇒ 无锁；同 hart 程序序保证读时指针恒
    ///   有效（写读互不同期）。未装槽 → null。
    ///
    /// 载荷语义：TaskIdent = 槽指向本核**在跑**的任务（trap 可信）；LastIdent =
    /// 末次身份记录（id/name/符号表；trap 不可信，见 [`ident`] 的 `Current::Last`）。
    /// 标签与指针同一原子字——载荷类型自描述，读侧无需第二读点（seat/shed
    /// 双写点无读撕裂窗口）。
    info: AtomicPtr<()>,
    /// 锁外：starved 长度镜像（steal 预检；与 inner 同结构体共生，不会分家）。
    /// **派生点唯一**：`starved_push` / `starved_pop` / `starved_remove` /
    /// `starved_clear` 四个方法体内。
    starved_len: AtomicUsize,
    /// 锁外：steal 起点游标（每次 steal 调用 fetch_add(1) % hart_count 拿起点）。
    /// 多核同时醒来时用本地游标派生不同起点——避免全从 hart 0 起步造成的 cache
    /// 热点（多 hart 同时对同一目标的 L1 锁 RMW → cache line 乒乓 = 雷鸣群）。
    /// per-hart 独立，每核 fetch_add 是 Relaxed 无需同步。
    steal_cursor: AtomicUsize,
}

/// 锁内核心：running（运行中，不在队列）+ starved（就绪队列，FIFO）。
pub(super) struct SchedulerInner {
    pub(super) running: Option<Arc<Task>>,
    /// 就绪队列。**改动只经 [`Scheduler`] 的四个 `starved_*` 方法**（计数镜像在同一处
    /// 派生），故对 core.rs 之外私有；跨文件只留 `starved_is_empty` 一个持锁读法。
    starved: VecDeque<Arc<Task>>,
}

impl SchedulerInner {
    /// 本核就绪队列是否空（持锁读；轮转 / 唯一任务判断用）。
    pub(super) fn starved_is_empty(&self) -> bool {
        self.starved.is_empty()
    }
}

impl Scheduler {
    /// 构造（boot 适配面按实际核数逐 hart 建）。
    pub(super) fn new(hart: usize) -> Scheduler {
        Scheduler {
            hart,
            inner: SpinLock::new_level(
                Level::Scheduler,
                SchedulerInner {
                    running: None,
                    starved: VecDeque::new(),
                },
            ),
            info: AtomicPtr::new(core::ptr::null_mut()),
            starved_len: AtomicUsize::new(0),
            steal_cursor: AtomicUsize::new(0),
        }
    }

    /// 锁外读：starved 长度（steal 预检；Relaxed 提示，旧读最多少偷一次）。
    fn get_len(&self) -> usize {
        self.starved_len.load(Ordering::Relaxed)
    }

    /// 从唯一事实来源（starved.len()）重派生计数——须在持 inner 锁时调用。
    fn set_len(&self, inner: &SchedulerInner) {
        self.starved_len
            .store(inner.starved.len(), Ordering::Relaxed);
    }

    // ── 就绪队列的四个改点：镜像在方法体内派生，队列与计数不可能分家 ──

    /// 队尾入队 + 派生计数。
    fn starved_push(&self, i: &mut SchedulerInner, task: Arc<Task>) {
        i.starved.push_back(task);
        self.set_len(i);
    }

    /// 队首出队 + 派生计数；空队列 → None。
    fn starved_pop(&self, i: &mut SchedulerInner) -> Option<Arc<Task>> {
        let t = i.starved.pop_front();
        self.set_len(i);
        t
    }

    /// 摘除指定下标 + 派生计数（kill 的 Starved 分支）。
    fn starved_remove(&self, i: &mut SchedulerInner, pos: usize) {
        i.starved.remove(pos);
        self.set_len(i);
    }

    /// 清空 + 派生计数（关机）。
    fn starved_clear(&self, i: &mut SchedulerInner) {
        i.starved.clear();
        self.set_len(i);
    }

    /// 队尾入队（spawn / 轮转 / 唤醒共用）：push + 派生计数。
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

    /// 队首出队（run / reap / park 共用）：派生计数；空队列返回 None。
    pub(super) fn pull(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.lock();
        self.starved_pop(&mut i)
    }

    /// steal 用：非阻塞取队首（锁外预检后调用）。None = 队列空或锁忙。
    fn try_pull(&self) -> Option<Arc<Task>> {
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
            let frame =
                &mut *(t.ident.frame.pa.expect("frame span has pa").as_usize() as *mut TrapContext);
            frame.kernel_sp = trap_stack_edge(self.hart);
            // S 态任务上台即写 tp = 本 hart PerHart 指针（内核自举任务与 supervisor
            // 域任务同此约定）：被抢占后的恢复路径直接 sret 回打断点（不再经任何
            // 内核任务 trampoline 重建 tp），tp 必须在上台时就绪。U 态任务的 tp 是
            // TLS，不写。
            if t.ident.team.space.kind().is_supervisor() {
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
        let pa = task.ident.frame.pa.expect("frame span has pa").as_usize();
        // 记身份（写点唯一）：Arc::into_raw 交出克隆的计数给槽持有；旧指针由本
        // 槽此前持有（本核独占写），按载荷类型标签（bit0）收回归还其计数。
        let prev = self.info.swap(
            Arc::into_raw(task.ident.clone()).cast_mut() as *mut (),
            Ordering::AcqRel,
        );
        if !prev.is_null() {
            let prev = prev as usize;
            // SAFETY: prev 是本槽上次 swap 存入的 Arc::into_raw 结果；swap 取走
            // 后槽对其不再持有，此处 from_raw 收回该份计数并释放（同 hart 程序序，
            // 无并发的本槽读写）。类型按标签位判定——标签与指针原子同行，无撕裂。
            if prev & LAST_TAG != 0 {
                unsafe {
                    drop(Arc::from_raw((prev & !LAST_TAG) as *const LastIdent));
                }
            } else {
                unsafe {
                    drop(Arc::from_raw(prev as *const TaskIdent));
                }
            }
        }
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
        // 装槽完成 → 载荷为 TaskIdent（Live：trap 可信）。AcqRel swap 已发布
        // prepare 写出的帧/任务状态；ident() 的 Acquire 配对。
        pa
    }

    /// 槽降级：身份载荷从 TaskIdent 换成 LastIdent（末次记录）。本核在跑任务
    /// 离核且不接续装槽（reap / park 无后继）时调用——trap 帧不可信（clear 即将
    /// 归还）。LastIdent 只留符号化最小集（id/name），**不持有团队/空间**
    /// ——团队 Arc 借此归零即回收，地址空间不再被 idle 核钉住（关机零泄漏审计
    /// 与「末次符号化」兼得）。
    ///
    /// # Safety
    /// 调用方须持有本核在跑任务的身份且本核独占写槽（同 hart——seat/shed
    /// 互斥的天然保证）；旧载荷必为未标签 TaskIdent。
    fn shed(&self, ident: &Arc<TaskIdent>) {
        let last = Arc::new(LastIdent {
            id: ident.id,
            name: ident.name,
            team: ident.team.name(),
        });
        let prev = self.info.swap(
            (Arc::into_raw(last) as usize | LAST_TAG) as *mut (),
            Ordering::AcqRel,
        );
        if !prev.is_null() {
            // SAFETY: 降级只在拥有在跑任务时发生——旧载荷必为未标签 TaskIdent。
            debug_assert_eq!(prev as usize & LAST_TAG, 0, "shed 旧载荷带标签");
            unsafe {
                drop(Arc::from_raw(prev as *const TaskIdent));
            }
        }
    }

    /// 关机清理：清空槽载荷（LastIdent/TaskIdent Arc 归还）——关机基线审计前
    /// 调用，否则每 hart 末次 LastIdent 计入块差集误报泄漏（已实证：4 hart =
    /// 4 个 48B 假泄漏）。
    pub(crate) fn clear_slot(&self) {
        let prev = self.info.swap(core::ptr::null_mut(), Ordering::AcqRel);
        if prev.is_null() {
            return;
        }
        let prev = prev as usize;
        // SAFETY: 同 seat/shed 的 prev 回收纪律（swap 取走即独占；关机单核）。
        if prev & LAST_TAG != 0 {
            unsafe {
                drop(Arc::from_raw((prev & !LAST_TAG) as *const LastIdent));
            }
        } else {
            unsafe {
                drop(Arc::from_raw(prev as *const TaskIdent));
            }
        }
    }

    /// 跨边界原语（messenger 三种过渡共用）：取走 running + 装下一 starved 或
    /// shed 槽位。返回 (取走的 Arc<Task>, Optional 下一帧 PA)。
    ///
    /// 锁纪律：内锁取 running / 弹 starved 后立即放；seat 重新取内锁。
    /// messenger 在两次取锁之间做自己的簿记（sites / holders / husks
    /// 各自 L3 锁，绝不持 L3 取 L1）。
    pub(crate) fn disown_and_install_next(&self) -> (Arc<Task>, Option<usize>) {
        let mut i = self.inner.lock();
        let task = i.running.take().expect("no running task");
        let ident = task.ident.clone();
        let next = self.starved_pop(&mut i);
        drop(i);
        let next_pa = if let Some(next) = next {
            let pa = next.ident.frame.pa.expect("frame span has pa").as_usize();
            self.seat(next);
            Some(pa)
        } else {
            self.shed(&ident);
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
    pub(super) fn rotate(&self, i: &mut SchedulerInner, mut cur: Arc<Task>) -> Arc<Task> {
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
            let pa = cur.ident.frame.pa.expect("frame span has pa").as_usize();
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

    // 注：park / wait / reap 三个 Scheduler 方法已移至 [`crate::work::room::messenger`]，
    // 任务"离开 running 槽"的所有过渡归 messenger 管理——它们借 Scheduler::disown_and_install_next
    // 跨边界原语完成槽位 settled，再在 messenger 域内做 sites / husks 簿记。
}

// 每核调度器表：boot 时按 DTB 实际核数从 frame 分配，Box::leak 进 OnceLock
// （MAX_HART_SLOTS=4096 仅为编译期 VA 窗口上限，不固定静态数组）。长度镜像随结构体共生。

// ── 核心：全局表（SCHEDULERS / 名册）──

pub(super) static SCHEDULERS: OnceLock<&'static [Scheduler]> = OnceLock::new();

/// 终末释放：halt 路径的关闭钩子——释放 scheduler 在**就绪队列**里持有的全部
/// task 引用，触发 MailHolds::drop 链透传 mail Arcs 归零（DockMeta::drop → 共享区帧还）。
///
/// 关闭顺序（conductor::halt → conductor::hooked）：
///   1. scheduler::rip              ← 本函数：就绪队列强制释放 → mail 透传
///   2. block::flush                 ← block 池冲洗
///   3. audit::check_baseline        ← 帧/block 基线核对
///
/// 注：本函数清 scheduler 持有的 Arc<Task>（就绪队列）+ info 槽。messenger 簿记
/// （sites / holders / husks）由 [`messenger::rip`] 清——本函数连调之。
///
/// **`running` 槽有意不清**（旧头注写「全部 task 引用」，与代码不符，此处改正为事实）：
/// 关机屏障（`conductor::halt` 等 `HALT_ARRIVED == hart_count`）保证的是**各核已到达**
/// halt，不保证没有核还在任务上下文里（已记账的旁枝：败者核继续跑任务，实测报
/// `user page fault without running task`）。此刻释放 running 的最后一个 Arc，等于在别人
/// 脚下的内核栈/trap 帧上归还内存。代价是：**真有核停在任务上下文**时，那一个任务的帧会
/// 留在类别账上——那是旁枝的账，不是本函数该擅自抹掉的。名册只存 `Weak`，不构成持有
/// （它的条目在最后一步统一放掉）。
pub(crate) fn rip() {
    // 清各 hart 的就绪队列（`running` 不动——理由见上）
    let Some(cs) = SCHEDULERS.get() else { return };
    for c in cs.iter() {
        let mut i = c.inner.lock();
        c.starved_clear(&mut i);
    }
    // 清 messenger 簿记（sites / holders / husks）
    messenger::rip();
    // 清 info 槽（原 shutdown_slots 职责）
    for c in cs.iter() {
        c.clear_slot();
    }
    // 名册**放最后**：强引用先全放掉（就绪队列 / 站点 / 躯壳 / 槽），名册里的弱引用才是
    // `ArcInner` 的最后一道门——放早了也白放（强引用还在，块归还不掉）。
    if let Some(r) = ROSTER.get() {
        r.lock().clear();
    }
}

pub(super) fn schedulers() -> &'static [Scheduler] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

/// 放行入队（`Task::release` 收尾）：入本核就绪队列 **+ 踢醒一个休眠核**。
///
/// 簿记（`Team.tasks`）、未放行容器（`Team.held`）、产生计数（PUSHED）与 trace 都在
/// `TaskBuilder::hold` 完成——**计数挂在产生处**，Held 被父域 kill 时
/// REAPED/PUSHED 仍配平（否则 `done()` 恒假，系统永不停机）。
///
/// 「踢醒」为什么不在 [`Scheduler::push`](Scheduler::push) 里：`messenger::rise`
/// 唤醒一批时**只在批量之后踢一次**（单 tick 的 IPI 量 O(N) → O(1)），并进去会让
/// 批量路径退化成 N 次踢。新任务出现是单点事件，故踢在这里。
pub(crate) fn push(task: Arc<Task>) {
    current().push(task);
    // 新任务出现：单点踢醒 1 个 WFI 休眠核（可 steal 取活；多核广播会触发
    // 雷鸣群，多 hart 同时抢源 L1 → cache 行乒乓）。
    conductor::kick();
}

// ── 核心：名册（全世界任务的 id → Weak<Task> 索引）──
//
// **一张表，不是每 hart 一张**：原先 by_id 是 `Scheduler` 的字段，而每张表都插全量
// 副本（入册要遍历所有 hart 各插一遍、没有第二条插入路径）⇒ 每张都是全世界的完整
// 拷贝：查表要遍历、快照把每个任务返回 H 份、每条查询成本随 hart 数放大。名册是
// 全局事实，故只有一张。
//
// 名字（用户裁决）：`enlist` 入册 / `muster` 点名 / `roster` 名册。对偶 `delist`（除名）
// 是**保留名、暂不实现**——名册里「条目在」这件事本身就是「这个 id 存在过」的唯一事实
// 源：「已回收」与「从未分配」靠它分开（`muster` 为 `None` ⇔ 从未入册）。除名会把这两态
// 重新糊在一起（A2 已在站点表上教过一遍：删掉承载事实的东西，就只剩墓碑）。
//
// 表只增不删 ⇒ 名册随运行增长；条目是 `Weak`，不钉住对象本体（`ArcInner` 的归还等
// 关机时的 [`rip`] 一次性放掉全部条目）。锁 = Level::L3，只经下面三个函数触及。

static ROSTER: OnceLock<SpinLock<HashMap<usize, Weak<Task>>>> = OnceLock::new();

fn roster_table() -> &'static SpinLock<HashMap<usize, Weak<Task>>> {
    ROSTER.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 入册：任务产生处一次性（`Task::hold` 末尾）。`Weak` 升级失败 = 任务已消失 =
/// 自动失效，无需显式清理。
pub(crate) fn enlist(id: usize, task: &Arc<Task>) {
    roster_table().lock().insert(id, Arc::downgrade(task));
}

/// 点名：按 id 取一个，**只出弱引用**——要强引用由调用方当场短升（于是「谁短暂持了
/// 强引用」摆在调用点上，而不是藏在查询函数里）。
///
/// `None` = **从未入册**（非法 id）；`Some` 升不起来 = 已消失（对象已回收）。
pub(crate) fn muster(id: usize) -> Option<Weak<Task>> {
    roster_table().lock().get(&id).map(Weak::clone)
}

/// 名册：全世界任务的弱引用，**每个任务恰好一次**（`gate` 的快照来源，boot 注入）。
pub(crate) fn roster() -> Vec<Weak<Task>> {
    roster_table().lock().values().map(Weak::clone).collect()
}

/// 从全部 hart 的 starved 队列摘除指定任务（kill 的 Starved 分支）。返回是否
/// 摘到。只持本 hart 的 inner(L1)，逐 hart 顺序取、不嵌套其它锁。
///
/// 注：`state` 的读取与容器动作不在一把锁里（读来自调用方），窗口内被别核 seat 走
/// ⇒ 这里返 false ⇒ 本次 kill 丢失（见 `docs/audit-flying-wires.md` §10.3 C1）。
pub(crate) fn remove_from_starved(target: &Arc<Task>) -> bool {
    for s in schedulers() {
        let mut i = s.inner.lock();
        if let Some(pos) = i.starved.iter().position(|t| Arc::ptr_eq(t, target)) {
            s.starved_remove(&mut i, pos);
            drop(i);
            return true;
        }
    }
    false
}

/// 定位指定任务当前 running 于哪个 hart（kill 的 Running 分支）。None = 不在
/// 任何核 running 槽。逐 hart 锁内 ptr_eq 比较（短暂持 L1）。
pub(crate) fn running_hart(target: &Arc<Task>) -> Option<usize> {
    for s in schedulers() {
        let i = s.inner.lock();
        if i.running.as_ref().is_some_and(|t| Arc::ptr_eq(t, target)) {
            let h = s.hart;
            drop(i);
            return Some(h);
        }
    }
    None
}

/// 执行核调度器（`tp → PerHart.scheduler` 直达，零索引——替代
/// `&schedulers()[hart_id()]` 的「读 id → 数组索引 → 取元素」三步）。
///
/// # Safety
/// 仅内核态调用；boot 期 `scheduler::boot::init` 已 `set_scheduler` 填充
/// （`machine::scheduler()` 的 Acquire 配对 Release store）。指向 SCHEDULERS
/// 数组元素，'static。
pub(crate) fn current() -> &'static Scheduler {
    // SAFETY: tp 直达读出的指针非空（boot 后恒填充）且指向 SCHEDULERS 元素。
    unsafe { &*(crate::machine::scheduler() as *const Scheduler) }
}

// 注：sites / holders / husks 三张表与 WakeKey / Site / Ticket / Waiter
// 类型已全部移至 [`crate::work::room::messenger`]——"任务不在 running 槽"的状态机归
// messenger 所有。详见 messenger 模块头注。

// ── 核心：取活 / 休眠 / 回收机制（内部）──

/// 非阻塞偷取：先读 starved_len（锁外原子读，S 态共享不失效缓存行）——空队列
/// 不做 RMW，避免对受害者锁行乒乓；有活才 try_lock（失败即跳过——victim 忙时
/// 不等待，无锁序规则）。锁内 pull 复查队列防竞态。
///
/// 起点随机化：每核持 `steal_cursor` 本地 fetch_add(1) % hart_count 派生起点，
/// 多核同时醒来时各 hart 起点天然分散——避免全从 hart 0 起步造成的 cache
/// 热点（多 hart 同时对同目标的 L1 锁 RMW → cache line 乒乓 = 雷鸣群）。
pub(super) fn steal() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    let n = machine::hart_count();
    if n <= 1 {
        return None;
    }
    // 每核独立游标派生起点：fetch_add 是 Relaxed，无内存序代价。
    let start = current().steal_cursor.fetch_add(1, Ordering::Relaxed) % n;
    for off in 0..n {
        let v = (start + off) % n;
        if v == me {
            continue;
        }
        if schedulers()[v].get_len() == 0 {
            continue;
        }
        let Some(task) = schedulers()[v].try_pull() else {
            continue;
        };
        trace::note(EventKind::Room(RoomEvent::Steal {
            tid: task.ident.id,
            src_hart: v,
        }));
        return Some(task);
    }
    None
}

/// 本 hart 进入 WFI 休眠（Idle 自环的「阻塞点」）。
///
/// 协议：置睡眠位 → 复查（防 push 漏唤醒）→ 全退出检查 → 睡到最近 tock → WFI。
/// 唤醒后：有任务 → 正常出口；到期假醒但无活 → 哑睡壳回睡（保持睡眠位、不打点不清位）。
pub(super) fn wait() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    conductor::sleep(me);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 push 漏唤醒
    let found = current().pull().or_else(steal);
    if let Some(task) = found {
        conductor::wake(me);
        return Some(task);
    }
    if conductor::done() {
        conductor::halt();
    }
    loop {
        // 每次决定重新睡下前，先复审全退出：halt 的 yell 会把本核从 WFI 拉起。
        // 若这里不归队 halt，而 redeem 又无可唤醒任务、steal 也无活，
        // 就会清 SSIP 后回睡，停机屏障将永远等不到本核的 HALT_ARRIVED。
        if conductor::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }

        let delta = match timer::due() {
            Some(t) => t.as_ticks().saturating_sub(clock::now().as_ticks()),
            None => WFI_FAR,
        };
        timer::beat(delta);
        // WFI：SSIP（IPI）/ STIP（定时器到期）挂起即唤醒——只唤醒不取中断（SIE=0）。
        // 注意：不再有清退应答点——RFENCE 由固件强制打断空闲核（含 WFI 态），
        // 目标核进 trap 执行 sfence，无需空闲核主动 sweep。
        unsafe {
            core::arch::asm!("wfi");
        }
        // timer 到期分派由 messenger 处理（票根 → 键 → 站点，一路）
        if messenger::redeem() {
            break;
        }
        // 假醒：也可能被 yell 的 IPI 唤来 steal（有活入队）——先复查取活，
        // 有任务即正常出口（睡眠位就在本分支清掉，见下）；真无活才保持睡眠位回睡。
        if let Some(task) = current().pull().or_else(steal) {
            conductor::wake(me);
            return Some(task);
        }
        // 哑睡壳（假醒无活）：保持睡眠位、不打点不清位，清残留 SSIP 后回睡。
        // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
        unsafe { sip::clear_ssoft() };
    }
    // 正常出口：清 SSIP（防残留位导致下次 WFI 立即重醒）与睡眠位
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
    unsafe { sip::clear_ssoft() };
    conductor::wake(me);
    None
}

// 注：redeem / wake / wipe / bury 已移至 [`crate::work::room::messenger`]：
// - redeem：按票认领到期登记（一段，不区分 park / wait / join）
// - wake / wipe：按唤醒源叫醒一个 / 放行全部
// - bury：排空躯壳队列 + 清理钩子

// ── 核心：当前任务身份（槽）──

/// 身份槽载荷：Live = 本核**在跑**任务（trap 可信）；Last = 末次身份记录
/// （id/name/符号表；trap **不可信**且类型上不可读）。trap 只经 Live 轴暴露——
/// 悬垂帧读取在类型层不可表达。
pub enum Current {
    Live(Arc<TaskIdent>),
    Last(Arc<LastIdent>),
}

/// 末次身份记录：降级时从 TaskIdent 复制（id / 角色名 / **域名字**），
/// **不含 team/space/trap**——团队 Arc 借此归零即回收整个地址空间。
pub struct LastIdent {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    /// 域名字（程序身份；内联定长，故仍不持 Team）。
    pub(crate) team: env::Name,
}

impl Current {
    pub fn id(&self) -> usize {
        match self {
            Current::Live(t) => t.id,
            Current::Last(l) => l.id,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Current::Live(t) => t.name,
            Current::Last(l) => l.name,
        }
    }

    /// 域名字（程序身份；诊断用）。
    pub fn team_name(&self) -> env::Name {
        match self {
            Current::Live(t) => t.team.name(),
            Current::Last(l) => l.team,
        }
    }

    /// trap 帧物理地址：仅 Live 轴可读（本核在跑任务，帧必活）；Last → None。
    pub fn trap(&self) -> Option<PhysAddr> {
        match self {
            Current::Live(t) => Some(t.frame.pa.expect("frame span has pa")),
            Current::Last(_) => None,
        }
    }

    /// Live 轴内层身份（trap 路径消费：envcall / 用户缺页 / 空间翻译必有 running
    /// 任务；Last → None——无 running 任务时这些路径必然走不到，由调用方 expect）。
    pub fn live(&self) -> Option<&TaskIdent> {
        match self {
            Current::Live(t) => Some(t),
            Current::Last(_) => None,
        }
    }
}

/// 本核任务身份：seat 装槽时定型（Live 载荷 TaskIdent）；reap / park 无后继
/// 降级（Last 载荷 LastIdent）；未装槽 → None。无锁：写 = 本核 seat/shed 的
/// 带标签指针 swap（AcqRel），读 = 本核 trap/panic（Acquire +
/// increment_strong_count）——载荷不可变 + 同 hart 程序序 ⇒ 非阻塞、不 panic、
/// 读恒有效，正常路径与崩溃现场同一入口。载荷类型自描述（标签位与指针同行），
/// 无第二读点、无读写撕裂窗口。
pub fn ident() -> Option<Current> {
    let all = SCHEDULERS.get()?;
    let raw = all[machine::hart_id()].info.load(Ordering::Acquire) as usize;
    if raw == 0 {
        return None;
    }
    if raw & LAST_TAG != 0 {
        // SAFETY: 标签标记 = LastIdent 载荷；槽持有者对 p 保有一份计数（Acquire
        // 与存入侧 AcqRel 配对，记录数据已发布）。同 hart 程序序下本核读时无
        // 并发 swap——increment 后再 from_raw 克隆，归还时计数一致。
        let p = (raw & !LAST_TAG) as *const LastIdent;
        unsafe {
            Arc::increment_strong_count(p);
            Some(Current::Last(Arc::from_raw(p)))
        }
    } else {
        // SAFETY: 未标签 = TaskIdent 载荷；计数协议同上一臂（Arc 数据经 AcqRel
        // swap 发布）。
        let p = raw as *const TaskIdent;
        unsafe {
            Arc::increment_strong_count(p);
            Some(Current::Live(Arc::from_raw(p)))
        }
    }
}
