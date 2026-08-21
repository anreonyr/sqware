// 任务调度器（多核 hart B1）：per-hart 调度锁 + 非阻塞 steal + 动态 trap 栈。
//
// 状态机（task.rs）：TaskState 回答"任务在哪 + 该状态的数据"——Running{ticks_left}
// （运行预算）/ Blocked{reason}（阻塞原因）/ Starved（就绪，预算耗尽等补给）/
// Reaped（僵尸，等延迟回收）。阻塞原因与预算作为状态载荷放在任务上；blocked
// 为 clock 句柄 → Task 映射（见下），reaped 退化为纯 Arc<Task> 索引。
//
// 时间片记账：新选中任务获得满额 TIME_SLICE 预算；run 时 Running 预算 > 1 →
// 递减续跑（不重排），== 1 → 转 Starved 轮转。主动让出（envcall YIELD）走
// starve：无视剩余预算立即轮转——抢占与让出各自独立。
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
// 锁纪律：inner = level 1 每核一把；Team.tasks = 3 与 Space.inner
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

use core::fmt::Write;

use crate::console::Sink;
use crate::lock::{Level, OnceLock, SpinLock};
use crate::machine;
use crate::memory::manager::addr::VirtAddr;
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trace::{self, EventKind, SchedEvent};
use crate::runtime::trampoline::{restore, trap_stack_top};
use crate::runtime::trap::arm_timer;
use crate::runtime::{clock, timer, watch};
use table::Fmt;

use super::tie;
use crate::work::unit::{
    space::Space,
    task::{BlockReason, Task, TaskState},
    team::Team,
};

// ── 核心：常量 ──

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」。
const WFI_FAR: u64 = 1 << 60;
/// sleep(tock) 的句柄分配器——调度器自管；park「先入簿、后 tock」闭合竞态。
static NEXT_PARK_HANDLE: AtomicUsize = AtomicUsize::new(0);
/// running_task_id 无任务时的哨兵返回值（诊断用）。
const NO_TASK_ID: usize = usize::MAX;

/// 新选中任务的满额时间片（量子数）。耗尽才轮转；定时器仍每量子打断，
/// 只是任务不再每量子切走。park 的 ticks 语义不受影响。
const TIME_SLICE: u32 = 8;

/// 任务打印时的任务名列宽（cell 对齐；超宽截断）。各任务打印点共享单一来源。
pub const NAME_W: usize = 16;

/// 把 Fmt 拼好的一行补换行，一次 flush 到控制台（无堆无锁）。
fn task_emit<const CAP: usize>(mut f: Fmt<CAP>) {
    let _ = writeln!(f);
    let mut sink = Sink;
    let _ = f.flush(&mut sink);
}

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
            inner: SpinLock::new_level(
                Level::Scheduler,
                SchedInner {
                    running: None,
                    starved: VecDeque::new(),
                },
            ),
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
        // debug: 同一任务不得重复入队（重复 push = 多容器强持有 → 后续 task_mut
        // 断言失败）。必查**所有核**的 starved——同任务跨核双入队是漏网形态。
        #[cfg(debug_assertions)]
        {
            for (h, s) in schedulers().iter().enumerate() {
                let Some(g) = s.inner.try_lock() else {
                    continue;
                };
                if g.starved.iter().any(|x| Arc::ptr_eq(x, &task)) {
                    panic!(
                        "push: duplicate enqueue of task #{} '{}' on hart {} (already queued on hart {h}, len {})",
                        task.id,
                        task.name,
                        self.hart,
                        g.starved.len()
                    );
                }
            }
        }
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
        trace::note(EventKind::Sched(SchedEvent::Starve { tid: cur.id }));
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
        #[cfg(debug_assertions)]
        probe_strong(&task, "park: running take");
        debug_assert!(
            matches!(task.state(), TaskState::Running { .. }),
            "running 容器里不是 Running 任务"
        );
        let wake_at = clock::now().add(duration).as_ticks();
        let mut f = Fmt::<64>::new();
        let _ = write!(f, "task #{} ", task.id);
        f.cell(task.name, NAME_W);
        let _ = write!(f, ": parked (wake @ {wake_at:#x})");
        task_emit(f);
        trace::note(EventKind::Sched(SchedEvent::Park {
            tid: task.id,
            wake_at: wake_at as usize,
        }));
        #[cfg(debug_assertions)]
        {
            probe_strong(&task, "park: before blocked insert");
        }
        task_mut(&mut task).transform(TaskState::Blocked {
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
    fn reap(&self) {
        let mut i = self.inner.lock();
        let mut exited = i.running.take().expect("no running task");
        debug_assert!(
            matches!(exited.state(), TaskState::Running { .. }),
            "running 容器里不是 Running 任务"
        );
        let mut f = Fmt::<64>::new();
        let _ = write!(f, "task #{} ", exited.id);
        f.cell(exited.name, NAME_W);
        let _ = write!(f, ": exited");
        task_emit(f);
        trace::note(EventKind::Sched(SchedEvent::Exit { tid: exited.id }));
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
    // debug: 强计数唯一 + 锁内 take/pop 互斥；Team.tasks 弱引用只作簿记、
    // 不读 Task 字段（见上）。
    // 断言失败 = 同一任务被多容器同时强持有（运行/就绪/阻塞/僵尸队列重复入队），
    // 打印身份与引用计数定位现场。
    #[cfg(debug_assertions)]
    {
        let sc = Arc::strong_count(t);
        if sc != 1 {
            // 真实调用者：task_mut 的入口 ra 保存在其帧槽 (s0-8)（0x8021f81a 的
            // `sd ra, 0x1e8(sp)`）。注意不能用 `mv ra`——编译器把 asm 挪到
            // Arc::strong_count 调用之后，读到的是该调用的返回地址。同一 asm 内
            // 同时读 ra/fp，保证两点同时刻。
            let ra: usize;
            let fp: usize;
            // SAFETY: 读返回地址与帧指针寄存器，纯读。
            unsafe {
                core::arch::asm!(
                    "mv {0}, ra",
                    "mv {1}, s0",
                    out(reg) ra,
                    out(reg) fp,
                    options(nomem, nostack, preserves_flags)
                );
            }
            // task_mut 入口：sd ra, 0x1e8(sp)；s0 = sp+0x1f0 → 槽 = s0-8。
            // (s0-16) = 上一帧 s0。栈上读回上一级返回地址做交叉验证。
            let caller_ra = unsafe { *(fp as *const usize).sub(1) };
            let prev_fp = unsafe { *(fp as *const usize).sub(2) };
            let caller2_ra = unsafe { *(prev_fp as *const usize).sub(1) };
            // 扫描全部容器，找出第二持有者（不分配——putln 直写 console）。
            let me = machine::hart_id();
            let ptr = Arc::as_ptr(t) as usize;
            putln!(
                "[dbg] task #{} '{}' ({:?}) strong_count {sc}: scanning containers...",
                t.id,
                t.name,
                t.state()
            );
            for (h, s) in schedulers().iter().enumerate() {
                match s.inner.try_lock() {
                    Some(g) => {
                        let is_me = if h == me { " [self]" } else { "" };
                        match &g.running {
                            Some(x) => {
                                putln!(
                                    "  hart {h} running:{is_me} #{} '{}' @ {:#x}{}",
                                    x.id,
                                    x.name,
                                    Arc::as_ptr(x) as usize,
                                    if Arc::as_ptr(x) as usize == ptr {
                                        " ** MATCH **"
                                    } else {
                                        ""
                                    }
                                )
                            }
                            None => putln!("  hart {h} running:{is_me} (none)"),
                        }
                        for (k, x) in g.starved.iter().enumerate() {
                            if Arc::as_ptr(x) as usize == ptr {
                                putln!("  hart {h}: ** starved[{k}] **");
                            }
                        }
                    }
                    // 本核自持锁（park/reap/rotate 内调 task_mut）与跨核真争用都表现为
                    // None——上一版无条件跳过 me，若第二持有者在**本核** running/starved
                    // 会被漏检；现在 try_lock 拿不到本核锁即打印提示，能拿到则照常扫描。
                    None => {
                        let why = if h == me {
                            "(self-held, skip)"
                        } else {
                            "(locked, skip)"
                        };
                        putln!("  hart {h}: {why}");
                    }
                }
            }
            // blocked 映射全量 dump（含非匹配条目——上一次只知道「无匹配」，无法
            // 排除「映射里就是没有」还是「并发被移除」）。
            if let Some(b) = blocked().try_lock() {
                putln!("  blocked map ({} entries):", b.len());
                for (k, x) in b.iter() {
                    putln!(
                        "    [{k:#x}] task #{} '{}' @ {:#x}{}",
                        x.id,
                        x.name,
                        Arc::as_ptr(x) as usize,
                        if Arc::as_ptr(x) as usize == ptr {
                            " ** MATCH **"
                        } else {
                            ""
                        }
                    );
                }
            } else {
                putln!("  blocked: (locked, skip)");
            }
            if let Some(r) = TASK_TABLES.reaped.try_lock() {
                putln!("  reaped queue ({} entries):", r.len());
                for (k, x) in r.iter().enumerate() {
                    putln!(
                        "    reaped[{k}] task #{} '{}' @ {:#x}{}",
                        x.id,
                        x.name,
                        Arc::as_ptr(x) as usize,
                        if Arc::as_ptr(x) as usize == ptr {
                            " ** MATCH **"
                        } else {
                            ""
                        }
                    );
                }
            } else {
                putln!("  reaped: (locked, skip)");
            }
            putln!("  == Arc ctrl block & task raw bytes ==");
            // ArcInner 布局：strong@data-16, weak@data-8（strong 在偏移 0、
            // weak 在偏移 8、data 在偏移 16）——上一版注释把标签写反了。
            // 注意 Task 结构体字段顺序是编译器重排过的（实测 state 在偏移 0、
            // id 在偏移 8），故 w0 是 state 判别值而非 id。
            // SAFETY: 读 t 指针附近内存做诊断；指针必须有效（任务存活）。
            unsafe {
                let s = *(ptr as *const usize).sub(2); // strong（真实）
                let w = *(ptr as *const usize).sub(1); // weak（真实）
                putln!("  arc@{ptr:#x}: hdr[strong]={s:#x} hdr[weak]={w:#x}");
                // Task 前 10 个字：state/id/name/team/trap（字段顺序编译器可重排）。
                for i in 0..10 {
                    putln!("    task[{i}] = {:#x}", *(ptr as *const usize).add(i));
                }
                putln!(
                    "  frame: ra={ra:#x} fp={fp:#x} caller_ra(real)={caller_ra:#x} prev_fp={prev_fp:#x} caller2_ra={caller2_ra:#x}"
                );
                // 粗糙栈回溯：沿当前 sp 扫 256 个字，凡落在 .text 范围者即调用返回地址。
                let mut sp: usize;
                core::arch::asm!("mv {0}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
                let mut n = 0usize;
                for i in 0..256 {
                    let v = *(sp as *const usize).add(i);
                    if (0x8020_0000..0x8060_0000).contains(&v) {
                        putln!("  trace[{i}] {v:#x}");
                        n += 1;
                        if n >= 12 {
                            break;
                        }
                    }
                }
            }
            panic!(
                "task #{} '{}' ({:?}): strong_count {sc} weak {} — 同一任务被多个容器强持有（caller via fp-walk={caller_ra:#x}）",
                t.id,
                t.name,
                t.state(),
                Arc::weak_count(t)
            );
        }
    }
    // SAFETY: 强计数唯一 + 锁内 take/pop 互斥；Team.tasks 弱引用只作簿记、
    // 不读 Task 字段（见上）。
    unsafe { &mut *(Arc::as_ptr(t) as *mut Task) }
}

/// debug: 容器间移动瞬时的强计数探针——任务应恒为唯一强持有（sc == 1）。
///
/// 调用点：run()/park()/steal() 把任务从一个容器移出的瞬间。sc > 1 说明同一
/// 任务已在另一容器/另一移动中被重复持有（映射双条目、双入队等）——全量 dump
/// 容器抓现行（help 诊断：panic 是 task_mut 的强计数断言，但第二持有者可能在
/// 更早的移动点就已出现；本探针把「出现点」前移）。
#[cfg(debug_assertions)]
fn probe_strong(t: &Arc<Task>, ctx: &str) {
    let sc = Arc::strong_count(t);
    if sc == 1 {
        return;
    }
    putln!(
        "[dbg] {ctx}: task #{} '{}' strong_count {sc} (expect 1) — duplicate holder!",
        t.id,
        t.name
    );
    let ptr = Arc::as_ptr(t) as usize;
    // 原始块 dump：ArcInner{strong@-16, weak@-8, data@ptr} + Task 前 16 字——
    // 判断块内存里是谁的数据（能否认出另一任务的 name/team/trap 特征）。
    unsafe {
        let s = *(ptr as *const usize).sub(2); // strong
        let w = *(ptr as *const usize).sub(1); // weak
        putln!("    arc@{ptr:#x}: hdr[strong]={s:#x} hdr[weak]={w:#x}");
        for i in 0..16 {
            putln!("      w[{i}] = {:#x}", *(ptr as *const usize).add(i));
        }
    }
    for (h, s) in schedulers().iter().enumerate() {
        match s.inner.try_lock() {
            Some(g) => {
                if let Some(x) = &g.running {
                    putln!(
                        "    hart {h} running: #{} '{}' @ {:#x}{}",
                        x.id,
                        x.name,
                        Arc::as_ptr(x) as usize,
                        if Arc::as_ptr(x) as usize == ptr {
                            " ** ME **"
                        } else {
                            ""
                        }
                    );
                }
                for (k, x) in g.starved.iter().enumerate() {
                    if Arc::as_ptr(x) as usize == ptr {
                        putln!("    hart {h}: ** starved[{k}] **");
                    }
                }
            }
            None => putln!("    hart {h}: (locked, skip)"),
        }
    }
    match blocked().try_lock() {
        Some(b) => {
            for (k, x) in b.iter() {
                putln!(
                    "    blocked[{k:#x}] task #{} @ {:#x}{}",
                    x.id,
                    Arc::as_ptr(x) as usize,
                    if Arc::ptr_eq(x, t) { " ** ME **" } else { "" }
                );
            }
        }
        None => putln!("    blocked: (locked, skip)"),
    }
    match TASK_TABLES.reaped.try_lock() {
        Some(r) => {
            for (k, x) in r.iter().enumerate() {
                putln!(
                    "    reaped[{k}] task #{} @ {:#x}{}",
                    x.id,
                    Arc::as_ptr(x) as usize,
                    if Arc::as_ptr(x) as usize == ptr {
                        " ** ME **"
                    } else {
                        ""
                    }
                );
            }
        }
        None => putln!("    reaped: (locked, skip)"),
    }
}

// 每核调度器表：boot 时按 DTB 实际核数从 frame 分配，Box::leak 进 OnceLock
// （MAX_HART_SLOTS=4096 仅为编译期 VA 窗口上限，不固定静态数组）。长度镜像随结构体共生。

// ── 核心：全局表（SCHEDULERS / blocked / reaped）──

static SCHEDULERS: OnceLock<&'static [Scheduler]> = OnceLock::new();

fn schedulers() -> &'static [Scheduler] {
    SCHEDULERS.get().expect("schedulers not initialized")
}

/// 全局容器：Blocked（睡眠映射 handle→Task）/ Reaped（回收队列）任务集合。
///
/// blocked 以 clock 的 deadline 句柄为键：条目即任务本身，阻塞原因（含 wake_at）
/// 在任务的 Blocked(Park) 载荷里，映射退化为「句柄 → 唯一 Arc<Task>」。唤醒
/// 由 timer::drain 产出到期句柄，unpark 按句柄摘除并唤醒。锁纪律与 Team.tasks
/// 同级（3）：park 路径 1 → 3 嵌套合法（blocked 与 clock 锁顺序获取、不嵌套）；
/// unpark 路径
/// 先放堆锁/队列锁再取调度锁（防 ABBA）。
/// Blocked（睡眠映射 handle→Task）— 惰性初始化：hashbrown 的 HashMap::new
/// 非 const，无法进 static（单独 static 保证 get_or_init 的 'static 借用）。
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
        #[cfg(debug_assertions)]
        probe_strong(&task, "steal: try_pop");
        let mut f = Fmt::<64>::new();
        let _ = write!(f, "hart {me}: stole task #{} ", task.id);
        f.cell(task.name, NAME_W);
        let _ = write!(f, " from hart {v}");
        task_emit(f);
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
    // 睡到最近 tock（无待唤醒则推远 stimecmp）：全核休眠时也能被最近唤醒点准时唤醒
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
    round();
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
        let mut f = Fmt::<64>::new();
        let _ = write!(f, "task #{} ", z.id);
        f.cell(z.name, NAME_W);
        let _ = write!(f, ": reaped reclaimed");
        task_emit(f);
        trace::note(EventKind::Sched(SchedEvent::Reap { tid: z.id }));
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

// 任务计数 / 全退出停机 / 休眠核唤醒：见 tie.rs。

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
    let tid = task.id;
    task.team.push_task(&task);
    schedulers()[me].push(task);
    tie::push();
    trace::note(EventKind::Sched(SchedEvent::Spawn { tid }));
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
    // 0. 多核 panic：警报已拉响且本 hart 非报警源 → 就地卧倒（不返回）。
    //    覆盖空闲/WFI 核经 wait() 在**内核态**处理 IPI 唤醒、不经过 trap_handler
    //    的路径（用户核走 trap 入口钩子）。常运行时恒 no-op。
    crate::runtime::halt::hush();
    watch::pulse();
    let me = machine::hart_id();
    let s = &schedulers()[me];
    let mut i = s.inner.lock();
    if let Some(mut cur) = i.running.take() {
        #[cfg(debug_assertions)]
        probe_strong(&cur, "run: running take");
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
        let prev_tid = cur.id;
        let next = s.rotate(&mut i, cur);
        let next_tid = next.id;
        drop(i);
        trace::note(EventKind::Sched(SchedEvent::Switch { prev_tid, next_tid }));
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

// ── 适配层：watch（值班看护）──────────────────────────────────────────

/// 打点 + 巡岗：pulse 后按现况判 A/L，命中即 raise。供 trap 定时器分支与
/// wait() 唤醒后调用（健康核的节拍）。
pub fn round() {
    watch::pulse();
    let probe = watch::Probe {
        has_work: has_work(),
        asleep: tie::waiting(),
    };
    if let Some(r) = watch::check(clock::now(), probe) {
        watch::raise(r);
    }
}

/// 是否有活：任一核 starved 非空，或 timer 有已到期 tock。
fn has_work() -> bool {
    for s in schedulers().iter() {
        if s.get_len() > 0 {
            return true;
        }
    }
    timer::next_tock().is_some_and(|t| t.as_ticks() <= clock::now().as_ticks())
}

/// 唤醒：从 clock 到期句柄按 blocked 映射摘除任务，Blocked → Starved 入本核
/// starved。调用方 = trap 定时器分支与 wait()（空闲核 WFI 唤醒后）。
///
/// 按 tock 堆（timer::drain）取到期者，与入队顺序无关。
/// 队列锁/堆锁先放后取，绝不持队列锁取调度锁（防 ABBA）。
pub fn unpark() {
    let due = timer::drain(clock::now());
    for handle in due {
        // blocked 映射锁：摘除（锁作用域到此语句结束即释放）
        let Some(mut task) = blocked().lock().remove(&handle) else {
            // 已取消/已由他路唤醒：跳过（timer 侧堆项已在 drain 丢弃）
            continue;
        };
        // debug: 移除后 strong 应为 1（映射条目是唯一强持有者）——若为 2 说明
        // 此刻还存在第二个持有者（比如映射里同名任务的双条目、或另一容器），
        // 全量 dump 映射抓现行。
        #[cfg(debug_assertions)]
        probe_strong(&task, "unpark: after blocked remove");
        task_mut(&mut task).transform(TaskState::Starved);
        let mut f = Fmt::<64>::new();
        let _ = write!(f, "task #{} ", task.id);
        f.cell(task.name, NAME_W);
        let _ = write!(f, ": woken");
        task_emit(f);
        trace::note(EventKind::Sched(SchedEvent::Wake { tid: task.id }));
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

/// 当前运行任务所属团队 Arc（锁内取、放锁返回）。
///
/// 供 envcall Spawn 使用：放锁后再建任务（spawn 内部逐段取 Space 锁 + scheduler 锁，
/// 不得跨锁持有）。持有的 Arc 保团队存活。无运行任务则 panic（envcall 必然有）。
pub fn running_team() -> Arc<Team> {
    let me = machine::hart_id();
    schedulers()[me]
        .inner
        .lock()
        .running
        .as_ref()
        .expect("no running task")
        .team
        .clone()
}

/// 当前运行任务所属团队（panic/诊断现场安全：try_lock + Option，不 panic、不阻塞）。
/// 与 running_team 不同，供崩溃转储符号化在可能持锁/无任务的 panic 现场调用。
pub fn running_team_try() -> Option<Arc<Team>> {
    let all = SCHEDULERS.get()?;
    let me = machine::hart_id();
    let guard = all[me].inner.try_lock()?;
    guard.running.as_ref().map(|t| t.team.clone())
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

/// 当前运行任务 id + 名称（panic 诊断用；非阻塞，失败返回 None）。
///
/// 与 `running_task_id` 不同，本函数专供 panic_handler：panic 路径故意绕过所有
/// 锁（见 halt.rs），故这里走两个防御——调度器尚未初始化（极早期 boot panic）
/// 直接返回 None；调度锁被 panic 现场持有（持锁处 panic）则 `try_lock` 拿不到
/// 立即放弃，避免递归死锁。拿到的 id/name 均为可拷贝数据，锁随作用域即放。
pub fn running_task_info() -> Option<(usize, &'static str)> {
    // 调度器未初始化（boot 早期 panic）时无可查。
    let all = SCHEDULERS.get()?;
    let me = machine::hart_id();
    let guard = all[me].inner.try_lock()?;
    guard.running.as_ref().map(|t| (t.id, t.name))
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
    // 回收 Reaped：此刻执行在 per-hart trap 栈上，不触碰任务内存；Reaped 任务
    // 不在任何核运行（running/starved 均无引用），任意核回收均安全。
    //
    // 必须在取活（可能触发 done→halt 的 check_baseline）**之前**清空 reaped 队列
    // ——否则最后退出的任务会带着它的栈/trap 帧及团队地址空间滞留到关机断言，
    // 泄漏成 check_baseline 的「task frames leaked」（reap 自身先入队，clear 后置
    // 时 run() 一旦走 done→halt 路径 clear 永不执行）。
    clear();
    // 取下一任务：此刻 running 已 take，本核空闲 → run（steal / WFI）
    run()
}
