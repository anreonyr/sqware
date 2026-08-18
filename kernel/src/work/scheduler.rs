// 任务调度器（多核 hart B1）：per-hart 调度锁 + 非阻塞 steal + 动态 trap 栈。
//
// 状态机（task.rs）：TaskState 回答"任务在哪 + 该状态的数据"——Running{ticks_left}
// （运行预算）/ Blocked{reason}（阻塞原因）/ Starved（就绪，预算耗尽等补给）/
// Reaped（僵尸，等延迟回收）。阻塞原因与预算作为状态载荷放在任务上；blocked
// 为 clock 句柄 → Task 映射（见下），reaped 退化为纯 Arc<Task> 索引。
//
// 时间片记账：新选中任务获得满额 TIME_SLICE 预算；run 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出（envcall YIELD）走
// starve：无视剩余预算立即轮转——抢占与让出不再是同构（旧实现二者共用 tick）。
//
// 退出回收：reap 标记 Reaped 入全局 reaped 容器（延迟回收——不能在自己正在用的
// 栈上回收自己）；本 hart 取到下一任务后 clear 统一回收。Reaped 任务
// 不在任何核运行，任意核回收均安全。
//
// 结构：Scheduler = inner(SpinLock) + starved_len(AtomicUsize 锁外镜像)。
// starved 字段私有，唯一修改路径是 push/pop（方法内持锁 + 从 starved.len()
// 派生计数）——不变量靠构造成立，无"漏一步 fetch_add/fetch_sub"的失步面。
// steal 锁外先读 starved_len 跳过空队列（不做 RMW），再 try_lock。
//
// 状态互斥：无原子字段。所有状态变更都经 task_mut（唯一 Arc 所有权 + &mut）——
// 锁内 take/pop 出任务 → Arc::get_mut；锁 + 所有权保证互斥，编译器强制。
//
// 锁纪律（lock/mod.rs）：inner = level 1 每核一把；Team.tasks = 3 与 Space.inner
// = 2 禁止嵌套——锁内只做纯 Vec 操作，绝不调 space 方法。blocked（sleepers 的
// handle→Task 映射）/ reaped / timer 的 TIMER_HEAP（tock 堆）同级（3）：park 路径 1 → 3
// 嵌套合法（顺序获取 blocked 与 clock 锁，绝不 3 → 3 嵌套）；unpark 路径先放
// 堆锁/队列锁再取调度锁（防 ABBA）。clear 只持 reaped 锁出队，放锁后再取
// Team.tasks / Space.inner（顺序获取，不嵌套）。
//
// 装槽（replace）：唯一装 running 的方法，自取锁，装前 running 必空（断言）。
// 调用方统一为「持锁 pop/rotate 出唯一 Arc<Task> → 放锁 → replace」——放锁
// 窗口内任务已出队且 strong_count == 1（唯一持有），steal 只偷仍在队列里的
// 任务，SIE=0 无中断重入，无并发别名。
//
// 文件布局：前半为调度核心（per-hart 结构 / 方法 / 全局表 / 取活·休眠·回收
// 机制）；后半为适配层——对外部模块（boot / create / trap / envcall）暴露的
// 入口函数，按服务模块分组。核心不依赖适配层，适配层只做「取本核 → 转发
// 核心方法」的薄封装。

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use riscv::register::sip;

use hashbrown::HashMap;

use crate::lock::{OnceLock, SpinLock};
use crate::machine;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::space::Space;
use crate::putln;
use crate::runtime::{clock, timer};
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::{restore, trap_stack_top};
use crate::runtime::trap::arm_timer;

use super::task::{BlockReason, Task, TaskState};
use super::tie;

// ── 核心：常量 ──

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」（替代原 TIMER_NEVER hack）。
const WFI_FAR: u64 = 1 << 60;
/// sleep(tock) 的句柄分配器——调度器自管；park「先入簿、后 tock」闭合竞态。
static NEXT_PARK_HANDLE: AtomicUsize = AtomicUsize::new(0);
/// running_task_id 无任务时的哨兵返回值（诊断用）。
const NO_TASK_ID: usize = usize::MAX;

/// 新选中任务的满额时间片（量子数）。耗尽才轮转——定时器频率不变（仍每量子
/// 打断），只是任务不再每量子切走；park 的 ticks 语义不受影响。
const TIME_SLICE: u32 = 8;

// ── 核心：per-hart 调度器结构与方法 ──

/// 每核调度器：真实数据在锁内，锁外只有 starved 长度镜像。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub struct Scheduler {
    /// 所属 hart（决定 trap 栈顶）。
    hart: usize,
    /// 锁内：running + starved（本核调度决策的原子单位）。
    inner: SpinLock<SchedInner>,
    /// 锁外：starved 长度镜像（steal 预检；与 inner 同结构体共生，不会分家）。
    starved_len: AtomicUsize,
}

/// 锁内核心：running（运行中，不在队列）+ starved（就绪队列，FIFO）。
struct SchedInner {
    running: Option<Arc<Task>>,
    starved: VecDeque<Arc<Task>>,
}

impl Scheduler {
    /// 构造（init_schedulers 按实际核数逐 hart 建）。
    const fn new(hart: usize) -> Scheduler {
        Scheduler {
            hart,
            inner: SpinLock::new(SchedInner {
                running: None,
                starved: VecDeque::new(),
            }),
            starved_len: AtomicUsize::new(0),
        }
    }

    /// 锁外读：starved 长度（steal 预检；Relaxed 提示，旧读最多少偷一次）。
    fn get_len(&self) -> usize {
        self.starved_len.load(Ordering::Relaxed)
    }

    /// 从唯一事实来源（starved.len()）重派生计数——须在持 inner 锁时调用。
    fn set_len(&self, inner: &SchedInner) {
        self.starved_len
            .store(inner.starved.len(), Ordering::Relaxed);
    }

    /// 队尾入队（spawn / 轮转 / 唤醒共用）：push + 派生计数。
    /// 只收 Starved 任务——容器 ⇔ 状态由断言强制。
    fn push(&self, task: Arc<Task>) {
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
    fn pop(&self) -> Option<Arc<Task>> {
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
        let t = task_mut(task);
        t.transform(TaskState::Running {
            ticks_left: TIME_SLICE,
        });
        // SAFETY: 帧 PA 恒等映射可写；帧属 task 独占（running 或刚从 starved 摘出）。
        unsafe {
            let frame = &mut *(t.trap.pa.as_usize() as *mut TrapContext);
            frame.kernel_sp = VirtAddr::from_raw(trap_stack_top(self.hart));
        }
        arm_timer(clock::duration_to_ticks(Duration::from_millis(100)));
    }

    /// 装槽：把 Starved 任务装为本 hart 的 running（自取锁）。装前 running 必空。
    ///
    /// 装槽为单一入口（合并自双安装路径）：锁内调用方（starve / park / run
    /// 的轮转分支）先放锁再调本方法——放锁窗口内任务已出队且唯一持有，安全
    /// （见模块头注释）。
    fn replace(&self, mut task: Arc<Task>) -> usize {
        let mut i = self.inner.lock();
        debug_assert!(i.running.is_none(), "装槽前 running 必须为空");
        self.prepare(&mut task);
        debug_assert!(
            matches!(task.state(), TaskState::Running { .. }),
            "running 容器只装 Running 任务"
        );
        let pa = task.trap.pa.as_usize();
        i.running = Some(task);
        pa
    }

    /// 轮转尾部（持锁、starved 非空）：Running → Starved 入队尾，队首上台。
    /// 调用方负责空队列判断（空 → 唯一任务续跑，不走本方法）。
    fn rotate(&self, i: &mut SchedInner, mut cur: Arc<Task>) -> Arc<Task> {
        task_mut(&mut cur).transform(TaskState::Starved);
        i.starved.push_back(cur);
        let next = i.starved.pop_front().expect("non-empty");
        self.set_len(i);
        next
    }

    /// 主动让出（envcall YIELD）：无视剩余预算立即轮转（Running → Starved）。
    fn starve(&self) -> usize {
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
        let next = self.rotate(&mut i, cur);
        drop(i);
        self.replace(next)
    }

    /// 当前任务 park：Running → Blocked(Park{wake_at})。入 timer 的 tock 堆
    /// （句柄 → blocked 映射），不再依赖入队顺序。返回下一帧 PA；本核 starved
    /// 空 → None（调用方转 run() 取活）。
    fn park(&self, duration: Duration) -> Option<usize> {
        let mut i = self.inner.lock();
        let mut task = i.running.take().expect("no running task");
        debug_assert!(
            matches!(task.state(), TaskState::Running { .. }),
            "running 容器里不是 Running 任务"
        );
        let wake_at = clock::now().add(duration).as_ticks();
        putln!(
            "task #{} '{}': parked (wake @ {wake_at:#x})",
            task.id,
            task.name
        );
        task_mut(&mut task).transform(TaskState::Blocked {
            reason: BlockReason::Park { wake_at },
        });
        // 注册 tock（防跨锁竞态，锁序 1 → 3 顺序获取、不嵌套）：
        //   NEXT_PARK_HANDLE（原子）→ blocked 入簿（blocked 锁）→ tock（timer 锁）。
        // 不变量：堆可见 ⇒ 簿记必在——unpark 按句柄摘除绝不会命中空簿记。
        let handle = NEXT_PARK_HANDLE.fetch_add(1, Ordering::Relaxed) as u64;
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
    fn reap(&self) {
        let mut i = self.inner.lock();
        let mut exited = i.running.take().expect("no running task");
        debug_assert!(
            matches!(exited.state(), TaskState::Running { .. }),
            "running 容器里不是 Running 任务"
        );
        putln!("task #{} '{}': exited", exited.id, exited.name);
        task_mut(&mut exited).transform(TaskState::Reaped);
        TASK_TABLES.reaped.lock().push_back(exited); // 1 → 3 合法
        tie::exit();
    }
}

/// 取唯一强引用下的 &mut Task。
///
/// 不变量：任务任一时刻只被一个容器强持有（running / starved / blocked / reaped
/// 恰好其一），Team 只持 Weak——strong_count == 1 恒成立。
/// 状态机更新因此由「锁内 take/pop 出唯一 Arc → &mut」保证互斥：没有锁内的
/// take/pop 拿不到 Arc，拿不到 Arc 就无法 &mut——编译器强制同步，无需原子字段。
///
/// # SAFETY（不用 Arc::get_mut 的原因）
///
/// `Arc::get_mut` 在 `weak_count > 0` 时也返回 None——而每个任务 spawn 时即被
/// `Team::push_task` 记入簿记（`Arc::downgrade`），weak_count ≥ 1 永不归零，
/// get_mut 恒失败。簿记弱引用**从不读 Task 字段**（push_task 只 downgrade、
/// prune_tasks 只 `ptr_eq` 比较），不构成可变访问冲突；互斥由锁 + strong_count
/// == 1（唯一强持有）保证，故直接经 `Arc::as_ptr` 转 `&mut` 安全。
fn task_mut(t: &mut Arc<Task>) -> &mut Task {
    debug_assert_eq!(
        Arc::strong_count(t),
        1,
        "task must be uniquely owned for mutation"
    );
    // SAFETY: 强计数唯一 + 锁内 take/pop 互斥；Team.tasks 弱引用只作簿记、
    // 不读 Task 字段（见上）。
    unsafe { &mut *(Arc::as_ptr(t) as *mut Task) }
}

// 每核调度器表：boot 时按 DTB 实际核数从 frame 分配，Box::leak 进 OnceLock
// （MAX_HARTS=8 仅为编译期安全上限，不固定静态数组）。长度镜像随结构体共生，
// 不再有平行数组 READY_LENS。

// ── 核心：全局表（SCHEDULERS / blocked / reaped）──

static SCHEDULERS: OnceLock<&'static [Scheduler]> = OnceLock::new();

fn schedulers() -> &'static [Scheduler] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

/// 全局容器：Blocked（睡眠映射 handle→Task）/ Reaped（回收队列）任务集合。
///
/// blocked 以 clock 的 deadline 句柄为键：条目即任务本身，阻塞原因（含 wake_at）
/// 在任务的 Blocked(Park) 载荷里，映射退化为「句柄 → 唯一 Arc<Task>」。唤醒
/// 由 timer::drain 产出到期句柄，unpark 按句柄摘除并唤醒——不再依赖入队
/// 顺序（修「后入睡更早醒沉队尾」缺陷）。锁纪律与 Team.tasks 同级（3）：park
/// 路径 1 → 3 嵌套合法（blocked 与 clock 锁顺序获取、不嵌套）；unpark 路径
/// 先放堆锁/队列锁再取调度锁（防 ABBA）。
/// Blocked（睡眠映射 handle→Task）— 惰性初始化：hashbrown 的 HashMap::new
/// 非 const，无法进 static（单独 static 保证 get_or_init 的 'static 借用）。
fn blocked() -> &'static SpinLock<HashMap<u64, Arc<Task>>> {
    static BLOCKED: OnceLock<SpinLock<HashMap<u64, Arc<Task>>>> = OnceLock::new();
    BLOCKED.get_or_init(|| SpinLock::new(HashMap::new()))
}

struct TaskTables {
    reaped: SpinLock<VecDeque<Arc<Task>>>,
}

static TASK_TABLES: TaskTables = TaskTables {
    reaped: SpinLock::new(VecDeque::new()),
};

// ── 核心：取活 / 休眠 / 回收机制（内部）──

/// 非阻塞偷取：先读 starved_len（锁外原子读，S 态共享不失效缓存行）——空队列
/// 不做 RMW，避免对受害者锁行乒乓；有活才 try_lock（失败即跳过——victim 忙时
/// 不等待，无锁序规则）。锁内 pop 复查队列防竞态。
fn steal() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    let n = machine::hart_count();
    for v in 0..n {
        if v == me {
            continue;
        }
        if schedulers()[v].get_len() == 0 {
            continue;
        }
        let Some(task) = schedulers()[v].try_pop() else {
            continue;
        };
        putln!(
            "hart {me}: stole task #{} '{}' from hart {v}",
            task.id,
            task.name
        );
        return Some(task);
    }
    None
}

/// 本 hart 进入 WFI 休眠（Idle 自环的「阻塞点」）。
///
/// 协议（闭合「置位后漏唤醒」窗口）：置睡眠位 → **复查**（自核队首 / steal——
/// 置位前的 push 已按位补 IPI，置位后的 push 会看到位）→ 全退出检查 →
/// 睡到最近 tock（tickless：由 timer::next_tock 推算，无则推远）→ WFI。唤醒后：
/// 若为定时器到期立即 drain 唤醒到期任务（含全核休眠场景）；清 SSIP 与睡眠位，
/// 返回 None 让外层循环重扫（也可能直接带出任务）。
fn wait() -> Option<Arc<Task>> {
    let me = machine::hart_id();
    tie::sleep(me);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 push 漏唤醒
    let found = schedulers()[me].pop().or_else(steal);
    if let Some(task) = found {
        tie::wake(me);
        return Some(task);
    }
    if tie::done() {
        tie::halt();
    }
    // 睡到最近 tock（无待唤醒则推远 stimecmp）——取代旧「blocked 非空才保持
    // 周期定时器」的 hack：全核休眠时也能被最近唤醒点准时唤醒
    let delta = match timer::next_tock() {
        Some(t) => t.as_ticks().saturating_sub(clock::now().as_ticks()),
        None => WFI_FAR,
    };
    arm_timer(delta);
    // WFI：SSIP（IPI）/ STIP（定时器到期）挂起即唤醒——只唤醒不取中断（SIE=0）
    unsafe {
        core::arch::asm!("wfi");
    }
    // 定时器到期唤醒：立即 drain（timer::drain）把到期任务送上本核 starved——
    // 空闲核也要能跑 unpark（原代码依赖其他核的周期中断，全核休眠会死等）
    unpark();
    // 唤醒后清 SSIP（防残留位导致下次 WFI 立即重醒）与睡眠位
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
    unsafe { sip::clear_ssoft() };
    tie::wake(me);
    None
}

/// 回收全部 Reaped 任务：簿记清理（Team.tasks 弱引用）+ 栈 slot/trap 帧归还 +
/// drop。安全：Reaped 任务不在任何核运行（running/starved 均无引用）。
/// 锁纪律：只持 reaped 锁出队，放锁后再取 Team.tasks / Space.inner（顺序获取，
/// 不嵌套——锁内绝不调 space 方法）。
fn clear() {
    loop {
        let Some(z) = TASK_TABLES.reaped.lock().pop_front() else {
            break;
        };
        putln!("task #{} '{}': reaped reclaimed", z.id, z.name);
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        z.team.prune_tasks(&z);
        // 锁外回收（Team.tasks 已放 → Space.inner=2 → FRAME=5 合法）
        z.team.space.task_reclaim(z.id, z.trap.va);
        drop(z);
    }
}

// ── 适配层：boot（boot.rs init / boot_main 调用）──

/// 按实际核数（DTB）动态分配 per-hart 调度器状态（boot 时调用**恰好一次**，
/// 先于任何调度器访问：spawn / trap / steal）。
pub fn init() {
    let n = machine::hart_count();
    assert!(n > 0, "no harts");
    let sched: Box<[Scheduler]> = (0..n).map(Scheduler::new).collect();
    assert!(
        SCHEDULERS.set(Box::leak(sched)).is_ok(),
        "schedulers double init"
    );
}

// 任务计数 / 全退出停机 / 休眠核唤醒：见 tie.rs（不变）。

/// 副核 idle 循环：spin + steal；拿到任务即 restore（永不返回）；全退出停机。
pub fn idle() -> ! {
    // restore 永不返回（切到用户态即离开内核）；拿不到任务就一直在
    // run() 的取活循环里 spin + steal，直到全退出停机。
    restore(run())
}

// ── 适配层：task（task.rs TaskBuilder::spawn 调用）──

/// 新任务入队收尾（task.rs TaskBuilder::spawn 调用）：入簿（Team.tasks，3）+
/// 入本 hart starved（1）+ PUSHED 计数；锁外唤醒 WFI 休眠核。
///
/// 锁序：Team.tasks 与调度锁顺序获取、不嵌套（无 3 → 1 方向）。
pub(crate) fn push(task: Arc<Task>) {
    let me = machine::hart_id();
    task.team.push_task(&task);
    schedulers()[me].push(task);
    tie::push();
    // 新任务出现：唤醒 WFI 休眠核（可 steal 取活）
    tie::wake_all();
}

// ── 适配层：trap（trap.rs 定时器 / 缺页 / 诊断调用）──

/// 统一入口（定时器抢占 / 取活）：running 预算 > 1 → 续跑（只减计数不重排）；
/// == 1 → 转 Starved 轮转；无 running → 取活（自核队首 → 跨核 steal → WFI）。
///
/// 取活与抢占合一：定时器分支（用户态陷阱，running 恒存在）走预算
/// 检查；enter_first_task / idle_loop / reap / park 空分支（running 已 take 或
/// 本为空闲）直接进入取活循环。
pub fn run() -> usize {
    let me = machine::hart_id();
    let s = &schedulers()[me];
    let mut i = s.inner.lock();
    if let Some(mut cur) = i.running.take() {
        let ticks_left = match cur.state() {
            TaskState::Running { ticks_left } => ticks_left,
            _ => unreachable!("running 容器里不是 Running 任务"),
        };
        if ticks_left > 1 {
            // 预算未耗尽：续跑——不切走、不进 starved
            task_mut(&mut cur).dec_ticks_left();
            let pa = cur.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        if i.starved.is_empty() {
            // 本 hart 唯一任务：预算耗尽但无处轮转，续跑
            let pa = cur.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        let next = s.rotate(&mut i, cur);
        drop(i);
        return s.replace(next);
    }
    drop(i);
    // 取活：Idle → Running 阻塞获取（自核队首 → 跨核 steal → 全退出检查 → WFI）
    loop {
        let me = machine::hart_id();
        if let Some(pa) = schedulers()[me].pop().map(|t| schedulers()[me].replace(t)) {
            return pa;
        }
        if tie::done() {
            tie::halt();
        }
        if let Some(task) = steal() {
            return schedulers()[me].replace(task);
        }
        if let Some(task) = wait() {
            return schedulers()[me].replace(task);
        }
    }
}

/// 唤醒：从 clock 到期句柄按 blocked 映射摘除任务，Blocked → Starved 入本核
/// starved。调用方 = trap 定时器分支与 wait()（空闲核 WFI 唤醒后）。
///
/// 缺陷修复：旧实现在 VecDeque 上只检查队首——后 park 的更早到期任务沉队尾
/// 永不唤醒；现按 tock 堆 (timer::drain) 取到期者，与入队顺序无关。
/// 队列锁/堆锁先放后取，绝不持队列锁取调度锁（防 ABBA）。
pub fn unpark() {
    let due = timer::drain(clock::now());
    for handle in due {
        // blocked 映射锁：摘除（锁作用域到此语句结束即释放）
        let Some(mut task) = blocked().lock().remove(&handle) else {
            // 已取消/已由他路唤醒：跳过（timer 侧堆项已在 drain 丢弃）
            continue;
        };
        task_mut(&mut task).transform(TaskState::Starved);
        putln!("task #{} '{}': woken", task.id, task.name);
        let me = machine::hart_id();
        schedulers()[me].push(task);
        tie::wake_all();
    }
}

/// 在当前运行任务的空间上执行闭包（锁内借出，引用不逃逸锁）。
pub fn with_running_space<R>(f: impl FnOnce(&Space) -> R) -> R {
    let me = machine::hart_id();
    let i = schedulers()[me].inner.lock();
    let task = i.running.as_ref().expect("no running task");
    f(&task.team.space)
}

/// 当前运行任务 id（诊断用；无任务返回 NO_TASK_ID）。
pub fn running_task_id() -> usize {
    let me = machine::hart_id();
    schedulers()[me]
        .inner
        .lock()
        .running
        .as_ref()
        .map(|t| t.id)
        .unwrap_or(NO_TASK_ID)
}

// ── 适配层：envcall（envcall.rs 分发调用）──

/// 主动让出入口（envcall YIELD 调用）：无视剩余预算立即轮转。
pub fn starve() -> usize {
    let me = machine::hart_id();
    schedulers()[me].starve()
}

/// 当前线程睡眠入口（envcall ENV_SLEEP/ENV_MSLEEP 分支调用）：交由 clock 换算
/// deadline → 方法 park；本核 starved 空 → run() 取活。
pub fn park(duration: Duration) -> usize {
    match schedulers()[machine::hart_id()].park(duration) {
        Some(pa) => pa,
        None => run(),
    }
}

/// 当前线程退出入口（envcall ENV_EXIT 分支调用）：标记 Reaped + 取下一任务
/// （run 的取活循环；拿不到就 WFI）；全部任务退出 → halt。
pub fn reap() -> usize {
    let me = machine::hart_id();
    schedulers()[me].reap();
    // 取下一任务：此刻 running 已 take，本核空闲 → run（steal / WFI）
    let pa = run();
    // 回收 Reaped：此刻执行在 per-hart trap 栈上，不触碰任务内存；Reaped 任务
    // 不在任何核运行（running/starved 均无引用），任意核回收均安全。
    clear();
    pa
}
