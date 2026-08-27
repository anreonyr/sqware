// 指令调度核心（conductor::core）— 多核任务调度：纯功能，无适配代码
//
// 时间片记账：新选中任务获得满额 TIME_SLICE 预算；run 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出走 starve：无视剩余
// 预算立即轮转——抢占与让出各自独立。
//
// 退出回收：reap 标记 Reaped 入全局 reaped 容器（延迟回收——不能在自己正在用的
// 栈上回收自己）；clear 统一回收。park/unpark 对偶：park 阻塞 + 登记 tock；
// unpark 取到期句柄摘除唤醒（由 trap 路径在 S-timer 处理时触发）。
//
// 结构：Conductor = inner(SpinLock) + starved_len(AtomicUsize 锁外镜像)。
// starved 字段私有，唯一修改路径是 push/pop（方法内持锁 + 从 starved.len()
// 派生计数）；steal 锁外先读 starved_len 跳过空队列（不做 RMW），再 try_lock。
//
// 状态互斥：无原子字段。所有状态变更都经 Task::exclusive（唯一 Arc 所有权
// + &mut，Arc::get_mut 的 weak≥1 变体）——锁内 take/pop 出任务 → 取 &mut；
// 锁 + 所有权保证互斥，编译器强制。
//
// 锁纪律：inner = level 1 每核一把；Team.tasks(3) 与 Space.inner(2) 禁止嵌套
// ——锁内只做纯 Vec 操作，绝不调 space 方法。blocked / reaped / timer 的 tock
// 堆同级（3）：park 路径 1 → 3 嵌套合法（绝不 3 → 3 嵌套）；unpark 路径先放
// 堆锁/队列锁再取调度锁（防 ABBA）。clear 只持 reaped 锁出队，放锁后再取
// Team.tasks / Space.inner（顺序获取，不嵌套）。
//
// 装槽（replace）：唯一装 running 的方法，自取锁，装前 running 必空（断言）。
//
// 可见性：`pub(super)` = 供本文件夹各适配面借用的核心表面（薄封装转发点）；
// `pub` = 供 conductor 之外消费（unpark —— trap 路径触发唤醒）。
// 核心不反向依赖任何适配面（wait 调 unpark，故 unpark 属核心而非适配）。

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use riscv::register::sip;

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::machine;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, SchedEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::room::tie;
use crate::work::unit::{
    space::SpaceKind,
    task::{BlockReason, Task, TaskState},
    team::Team,
};

// ── 核心：常量 ──

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」。
const WFI_FAR: u64 = 1 << 60;
/// sleep(tock) 的句柄分配器——调度器自管；park「先入簿、后 tock」闭合竞态。
static NEXT_PARK_HANDLE: AtomicUsize = AtomicUsize::new(0);
/// running_task_id 无任务时的哨兵返回值（诊断用）。
pub(super) const NO_TASK_ID: usize = usize::MAX;

/// 新选中任务的满额时间片（量子数）。耗尽才轮转；定时器仍每量子打断，
/// 只是任务不再每量子切走。park 的 ticks 语义不受影响。
const TIME_SLICE: u32 = 8;

// ── 核心：per-hart 调度器结构与方法 ──

/// 每核调度器：真实数据在锁内，锁外只有 starved 长度镜像。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub(super) struct Conductor {
    /// 所属 hart（决定 trap 栈顶）。
    hart: usize,
    /// 锁内：running + starved（本核调度决策的原子单位）。
    pub(super) inner: SpinLock<ConductorInner>,
    /// 锁外：running team 镜像（崩溃现场符号化）。与 `inner` 同结构体共生——
    /// 写侧 = 本核装槽窗口内派生（L1→L3 递增），读侧 = `running_team_try`
    /// 只 try_lock **本核**的这把小锁。per-hart 各一：报警核只读本核最近上台的
    /// team（idle 时保留旧值——符号化无碍）。
    pub(super) running_team: SpinLock<Option<Arc<Team>>>,
    /// 锁外：starved 长度镜像（steal 预检；与 inner 同结构体共生，不会分家）。
    starved_len: AtomicUsize,
}

/// 锁内核心：running（运行中，不在队列）+ starved（就绪队列，FIFO）。
pub(super) struct ConductorInner {
    pub(super) running: Option<Arc<Task>>,
    pub(super) starved: VecDeque<Arc<Task>>,
}

impl Conductor {
    /// 构造（boot 适配面按实际核数逐 hart 建）。
    pub(super) const fn new(hart: usize) -> Conductor {
        Conductor {
            hart,
            inner: SpinLock::new_level(
                Level::Scheduler,
                ConductorInner {
                    running: None,
                    starved: VecDeque::new(),
                },
            ),
            running_team: SpinLock::new_level(Level::L3, None),
            starved_len: AtomicUsize::new(0),
        }
    }

    /// 装槽时同步镜像（与 inner 同锁域调用：L1 调度锁内 → L3 镜像 严格递增）。
    fn mirror_team(&self, team: &Arc<Team>) {
        self.running_team.lock().replace(team.clone());
    }

    /// 锁外读：starved 长度（steal 预检；Relaxed 提示，旧读最多少偷一次）。
    fn get_len(&self) -> usize {
        self.starved_len.load(Ordering::Relaxed)
    }

    /// 从唯一事实来源（starved.len()）重派生计数——须在持 inner 锁时调用。
    fn set_len(&self, inner: &ConductorInner) {
        self.starved_len
            .store(inner.starved.len(), Ordering::Relaxed);
    }

    /// 队尾入队（spawn / 轮转 / 唤醒共用）：push + 派生计数。
    /// 只收 Starved 任务——容器 ⇔ 状态由断言强制。
    pub(super) fn push(&self, task: Arc<Task>) {
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
    pub(super) fn pop(&self) -> Option<Arc<Task>> {
        let mut i = self.inner.lock();
        let t = i.starved.pop_front();
        self.set_len(&i);
        t
    }

    /// steal 用：非阻塞取队首（锁外预检后调用）。None = 队列空或锁忙。
    fn try_pop(&self) -> Option<Arc<Task>> {
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
            let frame = &mut *(t.trap.pa.as_usize() as *mut TrapContext);
            frame.kernel_sp = trap_stack_edge(self.hart);
            // 内核任务上台即写 tp = 本 hart：被抢占恢复路径直接 sret 回打断点
            // （不经 ktask_trampoline 的 tp 重建），tp 必须在上台时就绪。
            if matches!(t.team.space.kind(), SpaceKind::Kernel) {
                frame.gpr.set_x(Gprs::TP, self.hart);
            }
        }
        timer::tick_after(clock::duration_to_ticks(Duration::from_millis(100)));
    }

    /// 装槽：把 Starved 任务装为本 hart 的 running（自取锁）。装前 running 必空。
    ///
    /// 安全前提：调用方先放锁再调本方法——放锁窗口内任务已出队且唯一持有
    /// （strong == 1），无并发别名。
    pub(super) fn replace(&self, mut task: Arc<Task>) -> usize {
        let mut i = self.inner.lock();
        debug_assert!(i.running.is_none(), "装槽前 running 必须为空");
        self.prepare(&mut task);
        debug_assert!(
            matches!(task.state(), TaskState::Running { .. }),
            "running 容器只装 Running 任务"
        );
        let pa = task.trap.pa.as_usize();
        self.mirror_team(&task.team);
        i.running = Some(task);
        pa
    }

    /// 轮转尾部（持锁、starved 非空）：Running → Starved 入队尾，队首上台。
    /// 调用方负责空队列判断（空 → 唯一任务续跑，不走本方法）。
    pub(super) fn rotate(&self, i: &mut ConductorInner, mut cur: Arc<Task>) -> Arc<Task> {
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
            let pa = cur.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        trace::note(EventKind::Sched(SchedEvent::Starve { tid: cur.id }));
        let next = self.rotate(&mut i, cur);
        drop(i);
        self.replace(next)
    }

    /// 当前任务 park：Running → Blocked(Park{wake_at})。入 timer 的 tock 堆
    /// （句柄 → blocked 映射）。返回下一帧 PA；本核 starved 空 → None（调用方
    /// 转 run() 取活）。
    pub(super) fn park(&self, duration: Duration) -> Option<usize> {
        let mut i = self.inner.lock();
        let mut task = i.running.take().expect("no running task");

        debug_assert!(
            matches!(task.state(), TaskState::Running { .. }),
            "running 容器里不是 Running 任务"
        );
        let wake_at = clock::now().add(duration).as_ticks();
        trace::note(EventKind::Sched(SchedEvent::Park {
            tid: task.id,
            wake_at: wake_at as usize,
        }));
        Task::exclusive(&mut task).transform(TaskState::Blocked {
            reason: BlockReason::Park { wake_at },
        });
        // 注册 tock（防跨锁竞态，锁序 1 → 3 顺序获取、不嵌套）：
        //   NEXT_PARK_HANDLE（原子）→ blocked 入簿（blocked 锁）→ tock（timer 锁）。
        // 不变量：堆可见 ⇒ 簿记必在——unpark 按句柄摘除绝不会命中空簿记。
        let handle = NEXT_PARK_HANDLE.fetch_add(1, Ordering::Relaxed) as u64;
        // debug: 同一任务不得在 blocked 簿记中重复登记（两次 park 同一任务 =
        // 唤醒后重复入队 → 多容器强持有）。持 blocked 锁遍历核对（锁内不做分配）。
        #[cfg(debug_assertions)]
        {
            let b = blocked().lock();
            if b.values().any(|t| Arc::ptr_eq(t, &task)) {
                panic!(
                    "park: task #{} '{}' already in blocked map (double park, handle {handle})",
                    task.id, task.name
                );
            }
        }
        blocked().lock().insert(handle, task);
        timer::tock(handle, wake_at);
        if i.starved.is_empty() {
            return None;
        }
        let next = i.starved.pop_front().expect("non-empty");
        self.set_len(&i);
        drop(i);
        Some(self.replace(next))
    }

    /// 当前任务退出：Running → Reaped 入全局 reaped 容器（延迟回收——不能在
    /// 自己正在用的栈上回收自己；计数在标记时完成）。
    pub(super) fn reap(&self) {
        let mut i = self.inner.lock();
        let mut exited = i.running.take().expect("no running task");
        debug_assert!(
            matches!(exited.state(), TaskState::Running { .. }),
            "running 容器里不是 Running 任务"
        );
        trace::note(EventKind::Sched(SchedEvent::Exit { tid: exited.id }));
        Task::exclusive(&mut exited).transform(TaskState::Reaped);
        TASK_TABLES.reaped.lock().push_back(exited); // 1 → 3 合法
        tie::exit();
    }
}

// 每核调度器表：boot 时按 DTB 实际核数从 frame 分配，Box::leak 进 OnceLock
// （MAX_HART_SLOTS=4096 仅为编译期 VA 窗口上限，不固定静态数组）。长度镜像随结构体共生。

// ── 核心：全局表（CONDUCTORS / blocked / reaped）──

pub(super) static CONDUCTORS: OnceLock<&'static [Conductor]> = OnceLock::new();

pub(super) fn conductors() -> &'static [Conductor] {
    CONDUCTORS.get().expect("conductors not initialized")
}

/// 全局容器：Blocked（睡眠映射 handle→Task）/ Reaped（回收队列）任务集合。
///
/// blocked 以 deadline 句柄为键：条目即任务本身，阻塞原因（含 wake_at）在任务
/// 的 Blocked(Park) 载荷里，映射退化为「句柄 → 唯一 Arc<Task>」；unpark 按句柄
/// 摘除并唤醒。锁级 3（与 Team.tasks 同级）：park 路径 1 → 3 嵌套合法（blocked
/// 与 clock 锁顺序获取、不嵌套）；unpark 先放堆锁/队列锁再取调度锁（防 ABBA）。
/// 惰性初始化：hashbrown 的 HashMap::new 非 const，无法进 static（单独 static
/// 保证 get_or_init 的 'static 借用）。
fn blocked() -> &'static SpinLock<HashMap<u64, Arc<Task>>> {
    static BLOCKED: OnceLock<SpinLock<HashMap<u64, Arc<Task>>>> = OnceLock::new();
    BLOCKED.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

struct TaskTables {
    reaped: SpinLock<VecDeque<Arc<Task>>>,
}

static TASK_TABLES: TaskTables = TaskTables {
    reaped: SpinLock::new_level(Level::L3, VecDeque::new()),
};

// ── 核心：取活 / 休眠 / 回收机制（内部）──

/// 非阻塞偷取：先读 starved_len（锁外原子读，S 态共享不失效缓存行）——空队列
/// 不做 RMW，避免对受害者锁行乒乓；有活才 try_lock（失败即跳过——victim 忙时
/// 不等待，无锁序规则）。锁内 pop 复查队列防竞态。
pub(super) fn steal() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    let n = machine::hart_count();
    for v in 0..n {
        if v == me {
            continue;
        }
        if conductors()[v].get_len() == 0 {
            continue;
        }
        let Some(task) = conductors()[v].try_pop() else {
            continue;
        };
        trace::note(EventKind::Sched(SchedEvent::Steal {
            tid: task.id,
            src_hart: v,
        }));
        return Some(task);
    }
    None
}

/// 本 hart 进入 WFI 休眠（Idle 自环的「阻塞点」）。
///
/// 协议（闭合「置位后漏唤醒」窗口）：置睡眠位 → **复查**（自核队首 / steal——
/// 置位前的 push 已按位补 IPI，置位后的 push 会看到位）→ 全退出检查 →
/// 睡到最近 tock（tickless：无到期 tock 则推远）→ WFI。唤醒后：有到期任务 →
/// 正常出口（清 SSIP 与睡眠位、round 打点、外层重扫）。
///
/// 护栏：睡距按 tickless 推算（无则推远 WFI_FAR）；到期假醒但无活 → 哑睡壳
/// 回睡：保持睡眠位、不打点不清位（假醒不伪装进展）。
pub(super) fn wait() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    tie::sleep(me);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 push 漏唤醒
    let found = conductors()[me].pop().or_else(steal);
    if let Some(task) = found {
        tie::wake(me);
        return Some(task);
    }
    if tie::done() {
        tie::halt();
    }
    loop {
        // 每次决定重新睡下前，先复审全退出：halt 的 yell 会把本核从 WFI 拉起。
        // 若这里不归队 halt，而 unpark 又无可唤醒任务、steal 也无活，就会清
        // SSIP 后回睡，停机屏障将永远等不到本核的 HALT_ARRIVED。
        if tie::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
            unsafe { sip::clear_ssoft() };
            tie::wake(me);
            tie::halt();
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
        if unpark() {
            break;
        }
        // 假醒：也可能被 yell 的 IPI 唤来 steal（有活入队）——先复查取活，
        // 有任务即正常出口（清位交外层）；真无活才保持睡眠位回睡。
        if let Some(task) = conductors()[me].pop().or_else(steal) {
            tie::wake(me);
            return Some(task);
        }
        // 哑睡壳（假醒无活）：保持睡眠位、不打点不清位，清残留 SSIP 后回睡。
        // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
        unsafe { sip::clear_ssoft() };
    }
    // 正常出口：清 SSIP（防残留位导致下次 WFI 立即重醒）与睡眠位
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
    unsafe { sip::clear_ssoft() };
    tie::wake(me);
    None
}

/// 唤醒：从到期句柄按 blocked 映射摘除任务，Blocked → Starved 入本核 starved。
///
/// 按 tock 堆取到期者（与入队顺序无关）；队列锁/堆锁先放后取，绝不持队列锁取
/// 调度锁（防 ABBA）。返回：本次是否撤出过任务（wait 的哑睡壳判定用）。
/// 由 trap 路径（S-timer 处理）在本 hart 触发；`pub`（conductor 之外消费）。
pub fn unpark() -> bool {
    let due = timer::drain(clock::now());
    let mut woke = false;
    for handle in due {
        // blocked 映射锁：摘除（锁作用域到此语句结束即释放）
        let Some(mut task) = blocked().lock().remove(&handle) else {
            // 已取消/已由他路唤醒：跳过（堆项随 drain 已丢弃）
            continue;
        };

        woke = true;
        Task::exclusive(&mut task).transform(TaskState::Starved);
        trace::note(EventKind::Sched(SchedEvent::Wake { tid: task.id }));
        let me = machine::hart_id();
        conductors()[me].push(task);
        tie::yell();
    }
    woke
}

/// 回收全部 Reaped 任务：簿记清理 + 栈 slot/trap 帧归还 + drop。安全：Reaped
/// 任务不在任何核运行（running/starved 均无引用）。锁纪律：只持 reaped 锁
/// 出队，放锁后再取 Team.tasks / Space.inner（顺序获取、不嵌套）。
pub(super) fn clear() {
    loop {
        let Some(z) = TASK_TABLES.reaped.lock().pop_front() else {
            break;
        };
        trace::note(EventKind::Sched(SchedEvent::Reap { tid: z.id }));
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        z.team.prune_tasks(&z);
        // 锁外回收（Team.tasks 已放 → Space.inner=2 → FRAME=5 合法）
        z.team.space.retire(z.id, z.trap.va);
        drop(z);
    }
}