// 指令调度核心（scheduler::core）— per-hart 调度：纯功能，无适配代码。
//
// 时间片记账：新选中任务获得满额 TIME_SLICE 预算；run 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出走 starve：无视剩余
// 预算立即轮转——抢占与让出各自独立。
//
// 结构：Scheduler = inner(SpinLock) + info(身份槽，无锁) + starved_len(AtomicUsize
// 锁外镜像)。info 槽 = `ident()` 的事实源：带标签指针（bit0 = 载荷类型：TaskIdent
// 在跑 / LastIdent 末次记录），写 = 本核 mount/demote 的 swap（AcqRel），读 = 本核
// trap/panic——同 hart 单写单读 + 载荷不可变 ⇒ 无锁（跨核读是 UB，字段私有且只经
// ident() 触及）。starved 字段私有，唯一修改
// 路径是 push/pull（方法内持锁 +
// 从 starved.len() 派生计数）；steal 锁外先读 starved_len 跳过空队列（不做 RMW），
// 再 try_lock。
//
// 状态互斥：无原子字段。所有状态变更都经 Task::exclusive（唯一 Arc 所有权
// + &mut，Arc::get_mut 的 weak≥1 变体）——锁内 take/pull 出任务 → 取 &mut；
// 锁 + 所有权保证互斥，编译器强制。
//
// 锁纪律：inner = level 1 每核一把；Team.tasks(3) 与 Space.inner(2) 禁止嵌套
// ——锁内只做纯 Vec 操作，绝不调 space 方法。task "离开 running" 的过渡（park /
// wait / reap）借 disown_and_install_next 跨边界原语交给 messenger 处理，本核
// 只负责 settled 槽位（Live=next 或 Last）；唤醒（drain_expired）也在 messenger。
//
// 装槽（mount）：唯一装 running 的方法，自取锁，空槽由 Option::replace 返回
// 旧值断言（绝不覆盖在跑任务）。装槽写 info 身份槽（TaskIdent 载荷）；降级
// （demote：reap / park 无后继）换 LastIdent 载荷——写点唯一 pair（同标签原子）。
//
// 可见性：`pub(super)` = 供本文件夹各适配面借用的核心表面（入口面转发点）；
// `pub` = 供 scheduler 之外消费（ident —— 身份槽读取）。wait() 是 WFI 入口
// 借 messenger::drain_expired 处理 timer 到期；clear_loop 不归本核管。

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use core::time::Duration;

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
use crate::work::unit::{
    elftable::ElfTable,
    space::SpaceKind,
    task::{Task, TaskIdent, TaskState},
};

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
    /// 本核装槽（mount，AcqRel swap，取走旧指针按标签收回归还其计数）或降级
    /// （demote，TaskIdent → LastIdent），读 = 本核 trap/panic 路径 load（Acquire）
    /// + increment_strong_count——载荷不可变 ⇒ 无锁；同 hart 程序序保证读时指针恒
    ///   有效（写读互不同期）。未装槽 → null。
    ///
    /// 载荷语义：TaskIdent = 槽指向本核**在跑**的任务（trap 可信）；LastIdent =
    /// 末次身份记录（id/name/符号表；trap 不可信，见 [`ident`] 的 `Current::Last`）。
    /// 标签与指针同一原子字——载荷类型自描述，读侧无需第二读点（mount/demote
    /// 双写点无读撕裂窗口）。
    info: AtomicPtr<()>,
    /// 锁外：starved 长度镜像（steal 预检；与 inner 同结构体共生，不会分家）。
    starved_len: AtomicUsize,
}

/// 锁内核心：running（运行中，不在队列）+ starved（就绪队列，FIFO）。
pub(super) struct SchedulerInner {
    pub(super) running: Option<Arc<Task>>,
    pub(super) starved: VecDeque<Arc<Task>>,
}

impl Scheduler {
    /// 构造（boot 适配面按实际核数逐 hart 建）。
    pub(super) const fn new(hart: usize) -> Scheduler {
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

    /// 队尾入队（spawn / 轮转 / 唤醒共用）：push + 派生计数。
    /// 只收 Starved 任务——容器 ⇔ 状态由断言强制。
    pub(crate) fn push(&self, task: Arc<Task>) {
        debug_assert_eq!(
            task.state(),
            TaskState::Starved,
            "starved 容器只收 Starved 任务"
        );
        let mut i = self.inner.lock();
        i.starved.push_back(task);
        self.set_len(&i);
    }

    /// 队首出队（run / reap / park 共用）：派生计数；空队列返回 None。
    pub(super) fn pull(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.lock();
        let t = i.starved.pop_front();
        self.set_len(&i);
        t
    }

    /// steal 用：非阻塞取队首（锁外预检后调用）。None = 队列空或锁忙。
    fn try_pull(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.try_lock()?;
        let t = i.starved.pop_front();
        self.set_len(&i);
        t
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
            // 内核任务上台即写 tp = 本 hart PerHart 指针：被抢占恢复路径直接 sret
            // 回打断点（不经 ktask_trampoline 的 tp 重建），tp 必须在上台时就绪。
            if matches!(t.ident.team.space.kind(), SpaceKind::Kernel) {
                frame
                    .gpr
                    .set_x(Gprs::TP, crate::machine::per_hart_ptr(self.hart));
            }
        }
        timer::tick_after(clock::duration_to_ticks(Duration::from_millis(100)));
    }

    /// 装槽：把 Starved 任务装为本 hart 的 running（自取锁）并记身份槽。空槽
    /// 由 `Option::replace` 返回旧值断言（绝不覆盖在跑任务）。
    ///
    /// 安全前提：调用方先放锁再调本方法——放锁窗口内任务已出队且唯一持有
    /// （strong == 1），无并发别名。
    pub(super) fn mount(&self, mut task: Arc<Task>) -> usize {
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
            matches!(task.state(), TaskState::Running { .. }),
            "running 容器只装 Running 任务"
        );
        // 装槽：replace 完成实际装槽（副作用不得藏在 debug_assert 内——release
        // 下断言被编译掉，装槽即失效 → running 恒空）。再断言旧槽必空（mount
        // 唯一装槽点）。
        let prev = i.running.replace(task);
        debug_assert!(prev.is_none(), "装槽前 running 必须为空");
        // 装槽完成 → 载荷为 TaskIdent（Live：trap 可信）。AcqRel swap 已发布
        // prepare 写出的帧/任务状态；ident() 的 Acquire 配对。
        pa
    }

    /// 槽降级：身份载荷从 TaskIdent 换成 LastIdent（末次记录）。本核在跑任务
    /// 离核且不接续装槽（reap / park 无后继）时调用——trap 帧不可信（clear 即将
    /// 归还）。LastIdent 只留符号化最小集（id/name/elftable），**不持有团队/空间**
    /// ——团队 Arc 借此归零即回收，地址空间不再被 idle 核钉住（关机零泄漏审计
    /// 与「末次符号化」兼得）。
    ///
    /// # Safety
    /// 调用方须持有本核在跑任务的身份且本核独占写槽（同 hart——mount/demote
    /// 互斥的天然保证）；旧载荷必为未标签 TaskIdent。
    fn demote(&self, ident: &Arc<TaskIdent>) {
        let last = Arc::new(LastIdent {
            id: ident.id,
            name: ident.name,
            elftable: ident.team.elftable.clone(),
        });
        let prev = self.info.swap(
            (Arc::into_raw(last) as usize | LAST_TAG) as *mut (),
            Ordering::AcqRel,
        );
        if !prev.is_null() {
            // SAFETY: 降级只在拥有在跑任务时发生——旧载荷必为未标签 TaskIdent。
            debug_assert_eq!(prev as usize & LAST_TAG, 0, "demote 旧载荷带标签");
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
        // SAFETY: 同 mount/demote 的 prev 回收纪律（swap 取走即独占；关机单核）。
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
    /// demote 槽位。返回 (取走的 Arc<Task>, Optional 下一帧 PA)。
    ///
    /// 锁纪律：内锁取 running / 弹 starved 后立即放；mount 重新取内锁。
    /// messenger 在两次取锁之间做自己的簿记（parked / sites / times / reaped
    /// 各自 L3 锁，绝不持 L3 取 L1）。
    pub(crate) fn disown_and_install_next(&self) -> (Arc<Task>, Option<usize>) {
        let mut i = self.inner.lock();
        let task = i.running.take().expect("no running task");
        let ident = task.ident.clone();
        let next = i.starved.pop_front();
        if next.is_some() {
            self.set_len(&i);
        }
        drop(i);
        let next_pa = if let Some(next) = next {
            let pa = next
                .ident
                .frame
                .pa
                .expect("frame span has pa")
                .as_usize();
            self.mount(next);
            Some(pa)
        } else {
            self.demote(&ident);
            None
        };
        (task, next_pa)
    }

    /// 当前 running 任务的帧 PA（messenger::wait 的 pend 消费路径用——不取走
    /// running，仅读取 frame 物理地址）。
    pub(crate) fn running_frame_pa(&self) -> Option<usize> {
        let i = self.inner.lock();
        i.running.as_ref().map(|t| {
            t.ident
                .frame
                .pa
                .expect("frame span has pa")
                .as_usize()
        })
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
        i.starved.push_back(cur);
        let next = i.starved.pop_front().expect("non-empty");
        self.set_len(i);
        next
    }

    /// 主动让出：无视剩余预算立即轮转（Running → Starved）。
    pub(super) fn starve(&self) -> usize {
        let mut i = self.inner.lock();
        let Some(cur) = i.running.take() else {
            panic!("starve with no running task on hart {}", self.hart);
        };
        if i.starved.is_empty() {
            // 本 hart 唯一任务：无需轮转，继续运行
            let pa = cur.ident.frame.pa.expect("frame span has pa").as_usize();
            i.running = Some(cur);
            return pa;
        }
        trace::note(EventKind::Room(RoomEvent::Starve { tid: cur.ident.id }));
        let next = self.rotate(&mut i, cur);
        drop(i);
        self.mount(next)
    }

    // 注：park / wait / reap 三个 Scheduler 方法已移至 [`crate::work::room::messenger`]，
// 任务"离开 running 槽"的所有过渡归 messenger 管理——它们借 Scheduler::disown_and_install_next
// 跨边界原语完成槽位 settled，再在 messenger 域内做 parked / sites / reaped 簿记。

}

// 每核调度器表：boot 时按 DTB 实际核数从 frame 分配，Box::leak 进 OnceLock
// （MAX_HART_SLOTS=4096 仅为编译期 VA 窗口上限，不固定静态数组）。长度镜像随结构体共生。

// ── 核心：全局表（SCHEDULERS / blocked / reaped）──

pub(super) static SCHEDULERS: OnceLock<&'static [Scheduler]> = OnceLock::new();

/// 终末释放：halt 路径的关闭钩子——强制释放 scheduler 持有的全部 task 引用，
/// 触发 MailHolds::drop 链透传 mail Arcs 归零（DockMeta::drop → 共享区帧还）。
///
/// 关闭顺序（conductor::halt → run_shutdown_hooks）：
///   1. scheduler::rip              ← 本函数：星等任务强制释放 → mail 透传
///   2. block::flush                 ← block 池冲洗
///   3. audit::check_baseline        ← 帧/block 基线核对
///
/// 注：本函数只清 scheduler 持有的 Arc<Task> + info 槽。messenger 簿记
/// （parked / sites / reaped）由 [`messenger::rip`] 清——本函数连调之。
pub(crate) fn rip() {
    // 清各 hart 内核的 starved 队列（running 不动——halt 时本 hart 不再调度）
    let Some(cs) = SCHEDULERS.get() else { return };
    for c in cs.iter() {
        c.inner.lock().starved.clear();
    }
    // 清 messenger 簿记（parked / sites / times / reaped）
    messenger::rip();
    // 清 info 槽（原 shutdown_slots 职责）
    for c in cs.iter() {
        c.clear_slot();
    }
}

pub(super) fn schedulers() -> &'static [Scheduler] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

/// 当前 hart running 任务的 Arc 克隆（mail 模块 push 接入点用）。envcall 边界
/// 必有 running 任务（trap 路径进入），返回 None 仅在 hart idle 时期——mail 适
/// 配面应在 idle 路径前不调此函数。
pub(crate) fn current_task() -> Option<Arc<Task>> {
    current().running_task()
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

// 注：parked / wait_sites / wait_times / reaped 四张表与 WaitKey / WaitSite / Waiter
// 类型已全部移至 [`crate::work::room::messenger`]——"任务不在 running 槽"的状态机归
// messenger 所有。详见 messenger 模块头注。

// ── 核心：取活 / 休眠 / 回收机制（内部）──

/// 非阻塞偷取：先读 starved_len（锁外原子读，S 态共享不失效缓存行）——空队列
/// 不做 RMW，避免对受害者锁行乒乓；有活才 try_lock（失败即跳过——victim 忙时
/// 不等待，无锁序规则）。锁内 pull 复查队列防竞态。
pub(super) fn steal() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    let n = machine::hart_count();
    for v in 0..n {
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
        // 若这里不归队 halt，而 drain_expired 又无可唤醒任务、steal 也无活，
        // 就会清 SSIP 后回睡，停机屏障将永远等不到本核的 HALT_ARRIVED。
        if conductor::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }

        let delta = match timer::next_tock() {
            Some(t) => t.as_ticks().saturating_sub(clock::now().as_ticks()),
            None => WFI_FAR,
        };
        timer::tick_after(delta);
        // WFI：SSIP（IPI）/ STIP（定时器到期）挂起即唤醒——只唤醒不取中断（SIE=0）
        unsafe {
            core::arch::asm!("wfi");
        }
        // timer 到期分派由 messenger 处理（sites + parked 两路）
        if messenger::drain_expired() {
            break;
        }
        // 假醒：也可能被 yell 的 IPI 唤来 steal（有活入队）——先复查取活，
        // 有任务即正常出口（清位交外层）；真无活才保持睡眠位回睡。
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

// 注：drain_expired / wake_by_event / clear_loop 三个函数已移至
// [`crate::work::room::messenger`]：
// - drain_expired：按 timer 到期分派到 sites / parked
// - wake_by_event：按事件键唤醒
// - clear_loop：排空 reaped 队列 + 清理钩子

// ── 核心：当前任务身份（槽）──

/// 身份槽载荷：Live = 本核**在跑**任务（trap 可信）；Last = 末次身份记录
/// （id/name/符号表；trap **不可信**且类型上不可读）。trap 只经 Live 轴暴露——
/// 悬垂帧读取在类型层不可表达。
pub enum Current {
    Live(Arc<TaskIdent>),
    Last(Arc<LastIdent>),
}

/// 末次身份记录：降级时从 TaskIdent 复制（id/name/符号表），**不含 team/space/
/// trap**——团队 Arc 借此归零即回收整个地址空间；符号表为 heap 分配，关机 flush
/// 冲掉——零泄漏审计与末次符号化兼得。
pub struct LastIdent {
    pub(crate) id: usize,
    pub(crate) name: &'static str,
    pub(crate) elftable: Option<Arc<ElfTable>>,
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

    /// 符号表：两臂通用（末次身份仍可符号化）。
    pub fn elftable(&self) -> Option<Arc<ElfTable>> {
        match self {
            Current::Live(t) => t.team.elftable.clone(),
            Current::Last(l) => l.elftable.clone(),
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

/// 本核任务身份：mount 装槽时定型（Live 载荷 TaskIdent）；reap / park 无后继
/// 降级（Last 载荷 LastIdent）；未装槽 → None。无锁：写 = 本核 mount/demote 的
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
