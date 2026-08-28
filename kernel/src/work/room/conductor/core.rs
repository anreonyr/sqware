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
// 结构：Conductor = inner(SpinLock) + info(身份槽，无锁) + starved_len(AtomicUsize
// 锁外镜像)。info 槽 = `ident()` 的事实源：带标签指针（bit0 = 载荷类型：TaskIdent
// 在跑 / LastIdent 末次记录），写 = 本核 mount/demote 的 swap（AcqRel），读 = 本核
// trap/panic——同 hart 单写单读 + 载荷不可变 ⇒ 无锁（跨核读是 UB，字段私有且只经
// ident() 触及）。starved 字段私有，唯一修改
// 路径是 push/pop（方法内持锁 +
// 从 starved.len() 派生计数）；steal 锁外先读 starved_len 跳过空队列（不做 RMW），
// 再 try_lock。
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
// 装槽（mount）：唯一装 running 的方法，自取锁，空槽由 Option::replace 返回
// 旧值断言（绝不覆盖在跑任务）。装槽写 info 身份槽（TaskIdent 载荷）；降级
// （demote：reap / park 无后继）换 LastIdent 载荷——写点唯一 pair（同标签原子）。
//
// 可见性：`pub(super)` = 供本文件夹各适配面借用的核心表面（入口面转发点）；
// `pub` = 供 conductor 之外消费（unpark —— trap 路径触发唤醒；ident —— 身份槽
// 读取）。核心不反向依赖任何适配面（wait 调 unpark，故 unpark 属核心而非适配）。

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use core::time::Duration;

use riscv::register::sip;

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::runtime::switcher::trap::trap_stack_edge;
use crate::work::room::tie;
use crate::work::unit::{
    elftable::ElfTable,
    space::SpaceKind,
    task::{BlockReason, Task, TaskIdent, TaskState, TrapFrame},
};

// ── 核心：常量 ──

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」。
const WFI_FAR: u64 = 1 << 60;
/// 身份槽载荷类型标签（bit0）：0 = TaskIdent（在跑任务），1 = LastIdent（末次
/// 记录）。标签与指针同一原子字——载荷类型自描述，读侧无需第二读点。
const LAST_TAG: usize = 1;
/// sleep(tock) 的句柄分配器——调度器自管；park「先入簿、后 tock」闭合竞态。
static NEXT_PARK_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// 新选中任务的满额时间片（量子数）。耗尽才轮转；定时器仍每量子打断，
/// 只是任务不再每量子切走。park 的 ticks 语义不受影响。
const TIME_SLICE: u32 = 8;

// ── 核心：per-hart 调度器结构与方法 ──

/// 每核调度器：真实数据在锁内，锁外只有身份槽与 starved 长度镜像。
///
/// repr(align(64))：相邻 hart 的锁 / 队列不落在同一缓存行（防假共享）。
#[repr(align(64))]
pub(super) struct Conductor {
    /// 所属 hart（决定 trap 栈顶）。
    hart: usize,
    /// 锁内：running + starved（本核调度决策的原子单位）。
    pub(super) inner: SpinLock<ConductorInner>,
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
            info: AtomicPtr::new(core::ptr::null_mut()),
            starved_len: AtomicUsize::new(0),
        }
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
            let frame = &mut *(t.ident.trap.pa.as_usize() as *mut TrapContext);
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
        let pa = task.ident.trap.pa.as_usize();
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
            let pa = cur.ident.trap.pa.as_usize();
            i.running = Some(cur);
            return pa;
        }
        trace::note(EventKind::Room(RoomEvent::Starve { tid: cur.ident.id }));
        let next = self.rotate(&mut i, cur);
        drop(i);
        self.mount(next)
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
        trace::note(EventKind::Room(RoomEvent::Park {
            tid: task.ident.id,
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
                    task.ident.id, task.ident.name
                );
            }
        }
        // 本核队列空 → 无后继装槽：先降级（task 其后被 insert 移动，须在此前
        // 取用其身份）。trap 不可信——身份非 running，帧不再保证存活。
        if i.starved.is_empty() {
            self.demote(&task.ident);
        }
        blocked().lock().insert(handle, task);
        timer::tock(handle, wake_at);
        if i.starved.is_empty() {
            return None;
        }
        let next = i.starved.pop_front().expect("non-empty");
        self.set_len(&i);
        drop(i);
        Some(self.mount(next))
    }

    /// 事件等待：Running → Blocked(Wait)。pend 存在 → 消费即回（不阻塞，无状态
    /// 变更）。`dur == Duration::MAX` → 永久（无 tock）；否则登记超时。
    /// 返回下一帧 PA；本核 starved 空 → None（调用方转 run() 取活）。
    pub(super) fn wait(&self, key: WaitKey, dur: Duration) -> Option<usize> {
        let mut i = self.inner.lock();
        let (wake_at, tock, next) = {
            let mut ws = wait_sites().lock();
            let site = ws.entry(key).or_insert(WaitSite {
                pend: false,
                waiters: VecDeque::new(),
            });
            if site.pend {
                // 闩消费：信号已至，任务不阻塞（续跑）
                site.pend = false;
                let pa = i
                    .running
                    .as_ref()
                    .expect("wait with no running task")
                    .ident
                    .trap
                    .pa
                    .as_usize();
                return Some(pa);
            }
            let mut task = i.running.take().expect("wait with no running task");
            let (wake_at, tock) = if dur == Duration::MAX {
                (None, None)
            } else {
                let wake_at = clock::now().add(dur).as_ticks();
                let handle = NEXT_PARK_HANDLE.fetch_add(1, Ordering::Relaxed) as u64;
                (Some(wake_at), Some(handle))
            };
            trace::note(EventKind::Room(RoomEvent::Wait {
                tid: task.ident.id,
                key: key.0,
            }));
            Task::exclusive(&mut task).transform(TaskState::Blocked {
                reason: BlockReason::Wait { wake_at },
            });
            // 本核队列空 → 无后继装槽：先降级（task 其后被移动，须在此前取用其
            // 身份）。trap 不可信——身份非 running，帧不再保证存活。
            if i.starved.is_empty() {
                self.demote(&task.ident);
            }
            site.waiters.push_back(Waiter { task, tock });
            let next = if i.starved.is_empty() {
                None
            } else {
                Some(i.starved.pop_front().expect("non-empty"))
            };
            (wake_at, tock, next)
        };
        drop(i);
        // 超时登记：先旁路簿记、后 tock（堆可见 ⇒ 簿记必在，同 park 纪律）
        if let (Some(wake_at), Some(handle)) = (wake_at, tock) {
            wait_times().lock().insert(handle, key);
            timer::tock(handle, wake_at);
        }
        next.map(|t| self.mount(t))
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
        trace::note(EventKind::Room(RoomEvent::Exit {
            tid: exited.ident.id,
        }));
        Task::exclusive(&mut exited).transform(TaskState::Reaped);
        // 离核且无后继装槽 → 槽降级 Last（先于入队：exited 其后被移动；clear 返回
        // 后即按 va 归还帧，trap 不可信）。团队 Arc 归零即回收——地址空间随释放。
        self.demote(&exited.ident);
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

/// 执行核调度器（`tp → PerHart.conductor` 直达，零索引——替代
/// `&conductors()[hart_id()]` 的「读 id → 数组索引 → 取元素」三步）。
///
/// # Safety
/// 仅内核态调用；boot 期 `conductor::boot::init` 已 `set_conductor` 填充
/// （`machine::conductor()` 的 Acquire 配对 Release store）。指向 CONDUCTORS
/// 数组元素，'static。
pub(super) fn current() -> &'static Conductor {
    // SAFETY: tp 直达读出的指针非空（boot 后恒填充）且指向 CONDUCTORS 元素。
    unsafe { &*(crate::machine::conductor() as *const Conductor) }
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

// ── 核心：事件等待（wait/wake）──

/// 事件等待键（newtype）：核心不解释组成，纯匹配。合成走 [`WaitKey::compose`]
/// （适配层在 envcall 边界调用，并入空间身份）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaitKey(usize);

impl WaitKey {
    /// 合成 = asid 高 16 位 || 用户 VA 低 48 位。单射需 va < 2^48（Sv39/48 满足；
    /// Sv57 若启用需重定布局）。与 fence::key 同源意：跨空间同 VA 不得混淆。
    pub fn compose(asid: usize, va: usize) -> WaitKey {
        WaitKey(((asid & 0xFFFF) << 48) | (va & ((1usize << 48) - 1)))
    }
}

/// 一个事件键的等待位：遗留信号（pend）+ 等待者队列。
struct WaitSite {
    /// 遗留信号（闩）：wake 无等待者 → 置位；wait 见位 → 消费即回（防漏唤醒）。
    pend: bool,
    /// 等待者（FIFO）；每项携带超时句柄（无超时 = None）。
    waiters: VecDeque<Waiter>,
}

struct Waiter {
    task: Arc<Task>,
    tock: Option<u64>,
}

/// 全局事件等待表（Level::L3，与 blocked 同级；绝不 3→3 嵌套）。
///
/// 单一注册表 = 单一线性化点：wait/wake 与超时到期都经本表锁摘除，败者见空即弃
/// （杜绝双唤醒）。键不主动销毁（pend 留滞为闩，语义见设计）。
fn wait_sites() -> &'static SpinLock<HashMap<WaitKey, WaitSite>> {
    static WAIT_SITES: OnceLock<SpinLock<HashMap<WaitKey, WaitSite>>> = OnceLock::new();
    WAIT_SITES.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 超时旁路：tock 句柄 → 事件键（timer 到期分派用）。任务本体只在 wait_sites
/// 注册；本表只放键（不含 Arc）——两条唤醒路仍都经 wait_sites 锁摘除。
fn wait_times() -> &'static SpinLock<HashMap<u64, WaitKey>> {
    static WAIT_TIMES: OnceLock<SpinLock<HashMap<u64, WaitKey>>> = OnceLock::new();
    WAIT_TIMES.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
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
    tie::sleep(me);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 push 漏唤醒
    let found = current().pop().or_else(steal);
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
        if let Some(task) = current().pop().or_else(steal) {
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
        // 事件等待超时：旁路表命中 → 从 wait-site 摘（by tock == handle）唤醒
        let wait_key = wait_times().lock().remove(&handle);
        if let Some(key) = wait_key {
            let popped = {
                let mut ws = wait_sites().lock();
                if let Some(site) = ws.get_mut(&key) {
                    if let Some(idx) = site.waiters.iter().position(|w| w.tock == Some(handle)) {
                        Some(site.waiters.remove(idx).expect("idx from position"))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(w) = popped {
                woke = true;
                let mut task = w.task;
                Task::exclusive(&mut task).transform(TaskState::Starved);
                trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
                current().push(task);
                tie::yell();
            }
            continue;
        }
        // park 到期（原路径）
        let Some(mut task) = blocked().lock().remove(&handle) else {
            // 已取消/已由他路唤醒：跳过（堆项随 drain 已丢弃）
            continue;
        };

        woke = true;
        Task::exclusive(&mut task).transform(TaskState::Starved);
        trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
        current().push(task);
        tie::yell();
    }
    woke
}

/// 事件唤醒：waiters 非空 → 唤醒队首（Blocked → Starved 推送本核 + yell）；
/// 空 → pend 置位（防漏唤醒）。返回是否唤到人。消费方 = utask/envcall；
/// 跨核唤醒经 steal 再平衡（与 unpark 一致）。
pub(super) fn wake(key: WaitKey) -> bool {
    let popped = {
        let mut ws = wait_sites().lock();
        let site = ws.entry(key).or_insert(WaitSite {
            pend: false,
            waiters: VecDeque::new(),
        });
        match site.waiters.pop_front() {
            Some(w) => Some(w),
            None => {
                site.pend = true;
                None
            }
        }
    };
    let Some(w) = popped else {
        return false;
    };
    // 摘超时旁路：堆项留至到期被 drain 空闲丢弃（不 untock——已 drain 的句柄再
    // untock 会永久污染 cancelled 表，见 drain 语义）
    if let Some(handle) = w.tock {
        wait_times().lock().remove(&handle);
    }
    let mut task = w.task;
    Task::exclusive(&mut task).transform(TaskState::Starved);
    trace::note(EventKind::Room(RoomEvent::Wake { tid: task.ident.id }));
    current().push(task);
    tie::yell();
    true
}

/// 回收全部 Reaped 任务：簿记清理 + 栈 slot/trap 帧归还 + drop。安全：Reaped
/// 任务不在任何核运行（running/starved 均无引用）。锁纪律：只持 reaped 锁
/// 出队，放锁后再取 Team.tasks / Space.inner（顺序获取、不嵌套）。
pub(super) fn clear() {
    loop {
        let Some(z) = TASK_TABLES.reaped.lock().pop_front() else {
            break;
        };
        trace::note(EventKind::Room(RoomEvent::Reap { tid: z.ident.id }));
        // 簿记清理（Team.tasks 锁；纯 Vec 操作——不变量：锁内不调 space 方法）
        z.ident.team.prune_tasks(&z);
        // 锁外回收（Team.tasks 已放 → Space.inner=2 → FRAME=5 合法）：栈 slot + trap 帧
        // 一次 with_flush 收回——PTE 清理 + 刷 TLB + 区间归还；帧随子 Map drop 归还。
        z.ident.team.space.with_flush(|inner| {
            if let Some((slot_va, slot_size)) = inner.stack.reclaim(z.ident.id) {
                inner.durable.unmap_frames(slot_va, slot_size);
            }
            if inner.frame.reclaim(z.ident.trap.va) {
                inner.durable.unmap_frames(z.ident.trap.va, PAGE_SIZE);
            }
        });
        drop(z);
    }
}

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

    /// trap 帧：仅 Live 轴可读（本核在跑任务，帧必活）；Last → None。
    pub fn trap(&self) -> Option<TrapFrame> {
        match self {
            Current::Live(t) => Some(t.trap),
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
    let all = CONDUCTORS.get()?;
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
