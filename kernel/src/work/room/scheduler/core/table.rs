// 全局表（core::table）— per-hart 调度器数组 + 名册 + 全机扫描 + 关机终末释放。
//
// 每核调度器表：boot 时按 DTB 实际核数从 frame 分配，Box::leak 进 OnceLock
// （MAX_HART_SLOTS=4096 仅为编译期 VA 窗口上限，不固定静态数组）。长度镜像随结构体共生。
//
// 名册与全机扫描都建在这张表上：`remove_from_starved` / `running_hart` 逐 hart 顺序
// 取放锁（只持 L1，不嵌套）、`rip` 关机时逐 hart 收队——三者都需要「全世界的核」，
// 故与表同居一处，而不是散进各入口面。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::room::conductor;
use crate::work::room::messenger;
use crate::work::unit::task::Task;

use super::hart::Scheduler;

pub(in super::super) static SCHEDULERS: OnceLock<&'static [Scheduler]> = OnceLock::new();

/// 终末释放：halt 路径的关闭钩子——释放 scheduler 在**就绪队列**里持有的全部
/// task 引用，触发 MailHolds::drop 链透传 mail Arcs 归零（DockMeta::drop → 共享区帧还）。
///
/// 关闭顺序（conductor::halt → conductor::hooked）：
///   1. scheduler::core::rip      ← 本函数：就绪队列强制释放 → mail 透传
///   2. block::flush                 ← block 池冲洗
///   3. audit::check_baseline        ← 帧/block 基线核对
///
/// 注：本函数清 scheduler 持有的 Arc<Task>（就绪队列）+ 身份槽。messenger 簿记
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
    // 清身份槽（原 shutdown_slots 职责）
    for c in cs.iter() {
        c.badge.clear();
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

/// 为本核就绪队列**预留**一格（放行路径不分配）。
///
/// # Errors
///
/// 队列无法扩容（内存耗尽）→ `Err(())`。
///
/// 与 [`try_reserve_roster`] 同旨：把唯一会分配的一步提到装配之前，失败时
/// 干净退回，让「生不出任务」是一个返回码而不是一次整机 halt。
pub(crate) fn try_reserve_starved(slot: usize) -> Result<(), ()> {
    current().try_reserve_starved(slot.saturating_add(1))
}

/// 放行入队（`Task::release` 收尾）：入本核就绪队列 **+ 踢醒一个休眠核**。
/// 簿记（`Team.tasks`）、未放行容器（`Team.held`）、产生计数（PUSHED）与 trace 都在
/// `TaskBuilder::hold` 完成——**计数挂在产生处**，Held 被父域 kill 时
/// REAPED/PUSHED 仍配平（否则 `done()` 恒假，系统永不停机）。
///
/// 「踢醒」为什么不在 [`Scheduler::push`] 里：`messenger::rise`
/// 唤醒一批时**只在批量之后踢一次**（单 tick 的 IPI 量 O(N) → O(1)），并进去会让
/// 批量路径退化成 N 次踢。新任务出现是单点事件，故踢在这里。
pub(crate) fn launch(task: Arc<Task>) {
    current().push(task);
    // 新任务出现：单点踢醒 1 个 WFI 休眠核（可 steal 取活；多核广播会触发
    // 雷鸣群，多 hart 同时抢源 L1 → cache 行乒乓）。
    conductor::kick();
}

// ── 名册（全世界任务的 id → Weak<Task> 索引）──
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

/// 为即将入册的**一条**预留名册容量。
///
/// # Errors
///
/// 容量扩不出来（内存耗尽）→ `Err(())`。
///
/// **为什么在产生处预留而不是让 `enlist` 失败**：名册插入是 `Spawn` 落库的中间
/// 一步，那里已经没有可返回的错误通道（任务对象已建、计数已记）。把唯一会分配
/// 的那一步提到**装配之前**——失败时干干净净地退回已领的栈/帧，`Spawn` 照旧答
/// `-4 OoM`。`try_reserve` 的语义正合此用：容量不够就报错，不做部分改动。
///
/// **不按 `id` 预留**：名册是 `HashMap<usize, Weak<Task>>`，容量是**元素数**的
/// 函数，与键的大小无关——而 `id` 来自全局 `NEXT_ID`，**只增不减**。先前用
/// `try_reserve(slot + 1)` 是把 `HashMap` 当 `Vec` 的按索引预留用：每产生一个
/// 任务就要求"再装得下 `id` 个"，于是预留量**随时间线性增长**，把一条恒定的
/// `O(1)` 需求变成随运行时长膨胀的开销，失败域被自己提前（实测：64M 下
/// `churn` 约 2220 轮即报 `-4 OoM`，而当时池里还有一万余帧）。
///
/// `try_reserve(1)` 才是这里真实的语义："马上要再插一个元素"。
pub(crate) fn try_reserve_roster() -> Result<(), ()> {
    roster_table().lock().try_reserve(1).map_err(|_| ())
}

/// 点名：按 id 取一个，**只出弱引用**——要强引用由调用方当场短升（于是「谁短暂持了
/// 强引用」摆在调用点上，而不是藏在查询函数里）。
///
/// `None` = **从未入册**（非法 id）；`Some` 升不起来 = 已消失（对象已回收）。
pub(crate) fn muster(id: usize) -> Option<Weak<Task>> {
    roster_table().lock().get(&id).map(Weak::clone)
}

/// 名册规模与**仍活着的条数**（`(总条数, 活条数)`）——audit 档的观测量。
///
/// 用 `Weak::strong_count()` 数活口：**只读、不升强引用**，故观测本身不会把被量对象
/// 拖住（`upgrade` 会 +1，用于观测就会改变被观测的事实）。关机时它应当是 `0`：全部任务
/// 都已回收，名册里不该还有强引用能升起来的条目。**它比帧/块计数更早说出问题的名字**
/// ——帧/块只告诉你"有东西没还"，它告诉你"哪个任务没走"。
#[cfg(feature = "audit")]
pub(crate) fn roster_live() -> (usize, usize) {
    let g = roster_table().lock();
    (g.len(), g.values().filter(|w| w.strong_count() > 0).count())
}

/// **在世任务的 id**（`strong_count > 0`），至多取 8 个：信标用它点名"谁还没走"。
///
/// 为什么返回定长数组而不是 `Vec`：本函数在**停机挂住**的现场被调用，而它要在
/// **持名册锁（L3）**时取数 —— 那时**不能分配**（分配会取 L2，L3→L2 嵌套即 lockdep
/// 违规，且在挂住的机器上分配未必成功）。故锁内只写定长数组，出锁后由调用方打印。
#[cfg(feature = "audit")]
pub(crate) fn roster_live_ids() -> (usize, [usize; 8]) {
    let g = roster_table().lock();
    let mut out = [0usize; 8];
    let mut n = 0usize;
    let mut more = 0usize;
    for (id, w) in g.iter() {
        if w.strong_count() == 0 {
            continue;
        }
        if n < 8 {
            out[n] = *id;
            n += 1;
        } else {
            more += 1;
        }
    }
    (more, out)
}

/// 名册：全世界任务的弱引用，**每个任务恰好一次**（`gate` 的快照来源，boot 注入）。
///
/// # 不 panic 的分配（本函数是**唯一**的快照来源，就在 `Spawn` 的路径上）
///
/// 旧版 `values().map(Weak::clone).collect()` 是一次**不可失败**的 `collect`：
/// 名册随任务数增长，`Vec` 扩容失败时 std 走 `handle_alloc_error` → `panic`
/// → **整机 halt**。实测现场（64M，`churn` 约 2260 轮）：
///
/// ```text
/// IllegalInstruction at sepc=<Vec<Weak<Task>>::from_iter> , stval=0x0
///   team 'shell' / task #4537 'u-thread'
/// ```
///
/// ——崩在**快照构建**里，而快照是 `Spawn` 必经的一步（`gate` 靠它认亲/级联）。
/// 与 `TaskBuilder::hold` 的簿记、`SpaceInner::maps` 同类：**簿记分配不得 panic**。
///
/// 失败时返回**空快照**而不是 `Err`：本函数在 `gate` 的查询面里（无错误通道），
/// 而空快照的语义是现成的、安全的——见 [`super::super::gate::snap`] 的头注：
/// 「未注入 ⇒ 空 ⇒ 查询退化为『找不到』，即**不级联、不认亲**」。即：内存耗尽
/// 时**放弃级联**，而不是停摆整机。
pub(crate) fn roster() -> Vec<Weak<Task>> {
    let g = roster_table().lock();
    let mut out: Vec<Weak<Task>> = Vec::new();
    if out.try_reserve(g.len()).is_err() {
        return Vec::new();
    }
    out.extend(g.values().map(Weak::clone));
    out
}

/// 从全部 hart 的 starved 队列摘除指定任务（kill 的 Starved 分支）。返回是否
/// 摘到。只持本 hart 的 inner(L1)，逐 hart 顺序取、不嵌套其它锁。
///
/// 注：`state` 的读取与容器动作不在一把锁里（读来自调用方），窗口内被别核 seat 走
/// ⇒ 这里返 false。**调用方（`messenger::doom::suspend`）据此重来**，重试耗尽按
/// `Running` 兜底（记 doomed + 定向 IPI）——所以「读到 Starved 却摘不到」不会静默
/// 丢掉这次 kill。
pub(crate) fn remove_from_starved(target: &Arc<Task>) -> bool {
    for s in schedulers() {
        let mut i = s.inner.lock();
        if s.starved_remove(&mut i, target) {
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
/// 取本核身份槽（[`super::ident::ident`]）也走这里：`boot::init` 先填每核直达指针、
/// 再发布本表 ⇒ **表在即指针在**，于是「取本核」只有一条路径。
///
/// # Safety
/// 仅内核态调用；boot 期 `scheduler::boot::init` 已 `set_scheduler` 填充
/// （`machine::scheduler()` 的 Acquire 配对 Release store）。指向 SCHEDULERS
/// 数组元素，'static。
pub(crate) fn current() -> &'static Scheduler {
    // SAFETY: tp 直达读出的指针非空（boot 后恒填充）且指向 SCHEDULERS 元素。
    unsafe { &*(crate::machine::scheduler() as *const Scheduler) }
}
