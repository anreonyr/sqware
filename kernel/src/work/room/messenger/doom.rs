// 扑杀（doom）——「杀」侧，**两阶段**：先停摆（摘出全部调度/等待
// 容器），再收尾（[`super::reap`]：钩子 → Reaped → 入躯壳队列）。
//
// 两个入口共用这一套：**级联**（父域退出 ⇒ 沿 heir 扑杀子树，[`doom`]）与
// **他杀**（`RoomCall::Doom` ⇒ [`cull`] 目标一个域，判据见 [`descends`]）。
//
// 两阶段是**正确性要求**，不是优化：钩子会摘门闩，摘门闩会唤醒等待者；若受害者
// 尚未停摆，它可能被别的核偷走并运行，在「已注定要死」的状态下观察到一个已死的
// 资源。他核 Running 任务无法被本核同步拉走（会破坏「Reaped 不在 running 槽」
// 不变量），故走 `doomed` 待杀集合：**兑现落在任务自己的时刻**（离核前自查 / timer
// 兜底 / 那一记定向 IPI 的落点），见 [`doomed_nudge`]。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::room::scheduler::core::muster;
use crate::work::unit::task::{Task, TaskState, TaskTag};
use crate::work::unit::team::Team;

use super::reap::reap;
use super::wait::holder::Ticket;
use super::wait::site::{SITE_SHARDS, WakeKey, shard_at};
use super::{prune, void};

/// 待杀集合（doomed）：**task id → 内核给的原因码**。
///
/// 为什么值不是"一个 bool"：退场原因码住的是**逐核暂存槽**
/// （[`super::set_exit_reason`]），写它的必须是"在**那颗核上**调用 `quit()` 的那段
/// 代码"；而被他杀的受害者是在**别的核**上被 IPI 唤起、自己在 `trap.rs` 里自退的
/// ——杀者写不进它的槽。原因码因此随杀令一起躺在集合里，由受害者那颗核取出来写进
/// **自己**的槽；不这样，被杀的域在 trace 里就是 `Exit { reason: 0 }`，与自愿退场同码。
///
/// "谁杀的"不在这里：下令者在下令那一刻就记了一笔（`RoomEvent::Doomed { tid, by }`），
/// 一个事实一份账，不随杀令再抄一份。
///
/// 无主簿记——只存 task_id 与原因码，不持 `Arc<Task>`（防「杀者撑着被杀者」）。
/// Level::L3，与站点表同级。
pub(super) fn doomed() -> &'static SpinLock<HashMap<usize, usize>> {
    static T: OnceLock<SpinLock<HashMap<usize, usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 表里**有几笔**待杀——只当**门**用。
///
/// 为什么要有它：查表是 L3 锁 + HashMap，而"看一眼自己是不是被判死了"落在**热路径**上
/// （每次离核：`park` / `wait` / `join` / `fall`，以及每个 timer tick）。杀人是极罕见的
/// 事件，故绝大多数时候这一眼只该花一次原子读。
///
/// 观察者纪律（B′）：这是**提示**，不是事实——`0` 的意思是"这一眼没货"，**结论**永远来自
/// 表锁内那一次 `remove`。故它不与插入配对成同步：计数**跟着表走**（插进新的一笔才加一、
/// 摘掉一笔才减一），中间那一瞬（已入表、计数未加）看到的 0 只会让人**少看一眼**——下一次
/// 检查点还会看，**不会错杀**。
pub(super) static PENDING: AtomicUsize = AtomicUsize::new(0);

/// 收令落在**哪一档**的计数（只读；停机读出口打）。
///
/// - `CULL_HELD` = 还在未放行的容器里（`Held`）；
/// - `CULL_STARVED` = **就绪但没被跑**（`Starved`：已被唤醒/放行，躺在某颗核的就绪队列里）——
///   "唤醒 ⇒ 上台"这一段的问题落在这一格；
/// - `CULL_BLOCKED` = 还挂在某个等待点上（`Blocked`）——**唤醒没送到**；
/// - `NUDGED` = 判它在台上（`Running`）⇒ 记一笔 doomed + 定向 IPI，等它自己在 `trap` 里退。
///
/// 这四个数才回答 rig A 要问的那一格："点名落在它**离核那一瞬**"中没中——`nudged > 0` 就是
/// 中过。台子的读数 `now`/`waited` **分不出位置**（那是"投递"与"复探"谁先到的赛跑；实测
/// 20 ms 台面下 `now=324/328`，而投递档靠这四个数才看得见）。
static CULL_HELD: AtomicUsize = AtomicUsize::new(0);
static CULL_STARVED: AtomicUsize = AtomicUsize::new(0);
static CULL_BLOCKED: AtomicUsize = AtomicUsize::new(0);
static NUDGED: AtomicUsize = AtomicUsize::new(0);

/// 上面那四格 `(未放行, 就绪未上台, 仍挂起, 在台上投递)`——停机读出口用。
pub(crate) fn branch_stats() -> (usize, usize, usize, usize) {
    (
        CULL_HELD.load(Ordering::Relaxed),
        CULL_STARVED.load(Ordering::Relaxed),
        CULL_BLOCKED.load(Ordering::Relaxed),
        NUDGED.load(Ordering::Relaxed),
    )
}

/// 关机终末释放（`messenger::rip`）：表与它的门一起清。
pub(super) fn rip() {
    doomed().lock().clear();
    PENDING.store(0, Ordering::Relaxed);
}

/// 停摆单线程：摘出全部调度/等待容器 → 置 `Doomed`。返 `true` = 本次停摆了它，
/// 调用方须随后 [`reap`]；`false` = 没动它（已 `Doomed`/`Reaped`、不在任何容器，
/// 或 `Running`——后者已记 doomed + 定向 IPI，待其自退时自己 `reap`）。
///
/// `reason` = 内核给的原因码：`Running` 分支把它随 doomed 一起留给目标核，目标核
/// 自退时写进**自己那颗核**的原因槽（杀者写不进那张槽，见 [`doomed`]）。
///
/// **观察者纪律**（B′）：本函数读的是**判别式**（[`Task::tag`]，原子），payload 在类型
/// 上够不着——`Blocked` 的键与票因此不是读出来的，而是**问站点表**（扫分片找这个 task
/// 的等待者，顺带拿到 key + ticket）。tag 只是**提示**，容器操作的返回值才是**结论**：
/// 两者不一致（窗口内被别核 `seat` 走 / 被 `wipe` 放行）就重来一轮，重试耗尽按
/// `Running` 兜底（记 doomed + 定向 IPI）——当场摘不掉时**不再静默丢掉这次 kill**。
/// 但"记一笔"不等于"注定自退"：兜底那一路的兑现条件与实测反例见 [`doomed_nudge`]。
///
/// 锁纪律：只持 L3 表，逐表取、放锁后再取下一表（L3 同层绝不嵌套）；锁内不 drop
/// Arc（drop 链触 L2）。
fn suspend(task: &Arc<Task>, reason: usize) -> bool {
    /// 窗口有多小都可能有：给几次重来，之后走兜底。
    const RETRY: usize = 4;

    for _ in 0..=RETRY {
        let taken = match task.tag() {
            TaskTag::Reaped | TaskTag::Doomed => return false,
            TaskTag::Held => {
                // 未放行的引导线程：从 Team.held **按身份**摘出（不是它就不动）。
                let team = task.ident.team.clone();
                let ok = team.release_held(task);
                if ok {
                    CULL_HELD.fetch_add(1, Ordering::Relaxed);
                }
                ok
            }
            TaskTag::Starved => {
                let ok = crate::work::room::scheduler::core::remove_from_starved(task);
                if ok {
                    // **就绪却一直没上台**——"唤醒 ⇒ 上台"这一段的问题落在这一格。
                    CULL_STARVED.fetch_add(1, Ordering::Relaxed);
                }
                ok
            }
            TaskTag::Blocked => {
                // 「读状态再摘容器」做不出来了 ⇒ 问容器：扫分片找它的等待者。
                let Some(ticket) = pop_waiter(task) else {
                    continue;
                };
                // 作废票根 + 消音到点（[`void`] 幂等）。
                void(ticket);
                // **还挂在等待点上**：唤醒没送到（或送到了又被重新挂起）。
                CULL_BLOCKED.fetch_add(1, Ordering::Relaxed);
                true
            }
            TaskTag::Running => {
                // 判它是"在跑"，却**找不到它在跑的那颗核**：那是"已离核、还没进容器"那道缝
                // （`block` 的 ③`swap` 与 ④入链之间）——缝里没有任何核能替我们收它，而它马上
                // 就会进容器 ⇒ 重来一轮（下一轮要么 `Blocked` 被站点表摘住，要么确有一颗核在
                // 跑它）。直接兜底会把这一笔记成一笔**没人兑现**的账，见 [`doomed_nudge`]。
                let hart = crate::work::room::scheduler::core::running_hart(task);
                if hart.is_none() {
                    continue;
                }
                return doomed_nudge(task, reason);
            }
        };
        if taken {
            // 它当时在某个容器里（不在台上）——具体哪一格已由上面各分支自己记。
            let mut t = task.clone();
            Task::exclusive(&mut t).transform(TaskState::Doomed);
            return true;
        }
        // 容器里没有它（tag 陈旧）：重来一轮重新分派。
    }
    // 重试耗尽：tag 一直在变（正在换容器）⇒ 按 Running 兜底。
    doomed_nudge(task, reason)
}

/// 站点表全扫：找 `task` 的那一环，摘下并**顺带拿到它的票**（票在容器里那一环上，
/// 链的强持有者就是那个站点；键在片内用来定位与 `prune`，票交调用方作废到点登记）。
/// 逐片取放，不持跨片锁；命中即摘即返。
fn pop_waiter(task: &Arc<Task>) -> Option<Ticket> {
    for shard in 0..SITE_SHARDS {
        let mut sites = shard_at(shard).lock();
        let mut hit: Option<(WakeKey, Ticket)> = None;
        for (key, site) in sites.iter_mut() {
            // 摘下那一环之后票从它身上读一次（读的人就是容器 + 持着分片锁，
            // 见 `Task::blocked_ticket`）。`task` 在本帧里是被强持有的 ⇒ 即使这里是
            // 最后一个引用也不会在锁内走 `Task::drop` 的 drop 链。
            if let Some(ticket) = site
                .remove_if(&mut |t| Arc::ptr_eq(t, task))
                .map(|mut node| Task::blocked_ticket(&mut node))
            {
                hit = Some((*key, ticket));
                break;
            }
        }
        // 摘空了就 prune（命中键留到循环外才用——循环内的 key 借着 sites）。
        let out = hit.map(|(key, ticket)| {
            prune(&mut sites, key);
            ticket
        });
        drop(sites);
        if out.is_some() {
            return out;
        }
    }
    None
}

/// `Running` 分支（也是重试耗尽的兜底）：记入待杀集合 + 定向 IPI，由目标核在陷阱里
/// 自查自退。找不到它的在跑核也照样记——记下这一笔至少不把 kill 丢掉。
///
/// # 兑现：这一记只是提示，兑现靠**五处**
///
/// 记一笔**不等于**兑现。查 `doomed` 的落点原先只有一处（`runtime::switcher::trap` 的
/// `SupervisorSoft` 分支），而且它只认「本核**当前**任务」⇒ 那一记 IPI 若被别的上下文
/// 取走（受害者在它到达之前自己离了核），这一笔就成了**孤儿**：全树再没人看它一眼，
/// 直到关机级联才被收掉（实测：`Doom` 答 `Ok`、目标 300 ms 内没自退，见 `scripts/soak.sh`）。
///
/// 故兑现改由**任务自己的时刻**兜底，IPI 退化成"让它尽快"的提示：
///
/// | 兑现点 | 盖住的形状 |
/// |---|---|
/// | 离核前自查（`wait::block` 的挂起路径） | 点名时它已经睡着 ⇒ 不必等谁来捅，自己就退 |
/// | `SupervisorTimer`：查本核当前任务 | 它被判死了却还在台上跑 |
/// | `SupervisorSoft`（此处投的那一记） | 尽量当场、尽快 |
/// | `SupervisorTimer`：[`sweep_doomed`] | 记在表里那几笔的**兜底**：不问"有没有人在跑它" |
/// | 空闲取活（`scheduler/core/fetch` 的 WFI 归来） | **整机闲着的形状**：定时器到期在空闲核上**不进陷阱**（WFI 时 SIE=0），只有这一条路能跑到兜底，见下 |
///
/// # 照实记（已收口）：缺的那个兑现点是**空闲取活**
///
/// 上面四格装上之后仍量到孤儿，率约 1/300。本轮用压测台 `rig A` 的**边界细扫**
/// （`d_us` ∈ [19.5, 20.5] ms、步 25 µs；受害者的"在台上"正好 20 ms ⇒ 点名正落在它
/// **离核那一瞬**）把它做成每轮 0~3 次的可测事件，再逐笔取证（`QEMU_SMP=4`、icount 关）：
///
/// | 读数 | 值 |
/// |---|---|
/// | 点名落点 | `nudge: hart=Some(1)`——IPI 投了，可落点核**已经没有这个任务**；或 `hart=None`（缝里 ⇒ 根本没有 IPI 可投） |
/// | 待杀表 | `reserve=true fresh=true pend=1` ⇒ 记录**记上了**（不是 OOM 丢掉） |
/// | 台子两次问 | 各**等满** 300 / 1000 ms（`join_ns=300440700 / 1000529800`），两次都答 `Unsettled` |
/// | 那 1.3 s 里扫单位 | **一次都没跑到**（`sweep=` 与 `orphan=` 皆 0，且 `TICKS` 计数一步没动 ⇒ 那段时间**没有一次定时器陷阱**） |
/// | 之后 | 机器一忙（下一次陷阱）当场 `pick: blocked` + `reap` |
///
/// **根因**：定时器到期在**空闲核**上不进陷阱——`fetch` 的 WFI 是 `SIE=0` 的（只唤醒、
/// 不取中断），到期由那一段代码自己调 `redeem()` 处理；而扫单位原先只挂在
/// `SupervisorTimer` 的**陷阱**分支上 ⇒ **整机闲着的时候，一笔"目标已离核"的杀令没有
/// 任何人会看一眼**。受害者随后挂起（`Blocked`），此后再没有"自己的时刻"（那次离核前
/// 自查在它入链之前就过去了），而那一记 IPI 又没有落点 ⇒ 只能等下一次陷阱。
///
/// **修法**：空闲取活那条路上也跑扫单位（`scheduler/core/fetch`，`redeem()` 旁边；
/// 代价是空表时一次 `PENDING` 原子读）。实测（同一台子、同一条命、边界细扫；读数取自
/// 当时带探针的树——探针已撤）：**修前 11 次 / 8 轮 → 修后 0 次 / 8 轮**。那 8 轮里有
/// 2 轮打出 `sweep:`（"跑到了、一笔没收掉"）⇒ 这条兜底不再是一档"闲置"的路；其余轮次
/// 只是没有记录活到扫单位那一步（`sweep:` 只在那一种情形下才打）。
///
/// **口径边界（照实）**：空闲且**没有任何到点登记**时核仍睡到"永远"（`WFI_FAR`，那条是
/// 裁决过的——不为它加有界拍）。那种极端形状下无人在场的杀令仍要等下一次中断；本修法
/// 保证的是"**只要核因任何原因醒来，扫单位就在那条路上跑**"。
fn doomed_nudge(task: &Arc<Task>, reason: usize) -> bool {
    // **在台上**那一档（也含重试耗尽的兜底）：记一笔 doomed + 定向 IPI，等它自己退。
    NUDGED.fetch_add(1, Ordering::Relaxed);
    let mut d = doomed().lock();
    // 扩容先试、失败即放弃这一笔（返回值本来就把'没记上'算进语义：不记才会把这次
    // kill 丢掉，故这里**先备后插**，备不出来也不 panic —— 整机照旧活着）。
    if d.try_reserve(1).is_ok() && d.insert(task.ident.id, reason).is_none() {
        // 门跟着表走：只在**新**记一笔时加一（重复点名不加）。
        PENDING.fetch_add(1, Ordering::Relaxed);
    }
    drop(d);
    let hart = crate::work::room::scheduler::core::running_hart(task);
    if let Some(hart) = hart {
        crate::work::room::conductor::nudge(hart);
    }
    false
}

/// 扫单位：把**还躺在待杀表里**的那几笔挨个再问一遍"现在收得掉吗"，收得掉就收掉。
/// 由 `SupervisorTimer` 每 tick 调（见 [`doomed_nudge`] 的"兑现"一节）。
///
/// 为什么它**理应**盖住其余所有形状：命令**记在表里**，故不必等"谁正好在台上"。一个已经
/// 挂起的孤儿在站点表里挂得好好的（`suspend` 按 `Blocked` 扫分片就能摘下它），一个还在跑
/// 的由本核/tick 那一处接住，一个名册里已经没有的（已被别的路收掉）则把这一笔作废。
/// **但它今天是闲置的**：装了探针的 140 轮里一次也没被用到（记录总在 tick 之前就被
/// SSIP 或那两处自查吃掉）——口径见 [`doomed_nudge`] 的照实记。
///
/// 顺带它是**陈旧记录的收尸人**：同一笔可能被别的路先收掉（级联 / 别核的扑杀），那时
/// 任务已是 `Reaped` 或已出名册 ⇒ 记录作废，`PENDING` 跟着回落（门不该被陈账顶开）。
///
/// 锁纪律：锁内只抄 `(tid, reason)`（**定长数组**——tick 里不分配），出锁才 `muster` /
/// `suspend` / `reap`（它们会取站点表等 L3 表，锁内碰即嵌套）。返本次收掉几笔。
pub(crate) fn sweep_doomed() -> usize {
    /// 一轮最多处理几笔：杀人是极罕见的事件，几笔足够；剩下的留给下一 tick。
    const BATCH: usize = 4;

    if PENDING.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    let mut batch = [(0usize, 0usize); BATCH];
    let mut n = 0;
    {
        let d = doomed().lock();
        for (tid, reason) in d.iter() {
            if n == BATCH {
                break;
            }
            batch[n] = (*tid, *reason);
            n += 1;
        }
    }
    let mut swept = 0;
    for &(tid, reason) in &batch[..n] {
        // 名册里没有它（已回收 / 从未入册）：这一笔作废（`take_doomed` 顺带落门）。
        let Some(task) = muster(tid).and_then(|w| w.upgrade()) else {
            let _ = take_doomed(tid);
            continue;
        };
        // 已经收尾了（别的路收的）：记录作废。
        if task.tag() == TaskTag::Reaped {
            let _ = take_doomed(tid);
            continue;
        }
        // 停摆得掉（挂在容器里 / 未放行 / 排队中）就收掉，并把这一笔摘掉——**取即清**；
        // 停摆不掉（还在跑 / 已被别核停摆）就留着，下一 tick 再看。
        if suspend(&task, reason) {
            reap(task);
            let _ = take_doomed(tid);
            swept += 1;
        }
    }
    swept
}

/// 扑杀整棵血缘子树（**两阶段**）：
///
///   1. 收集：显式工作栈沿 heir 收齐全部任务（防爆栈）；
///   2. 停摆：逐个 [`suspend`]——`Running` 分支只记 doomed + SSIP，它自退时自己 `reap`；
///   3. 收尾：逐个 [`reap`]（钩子 → Reaped → 入躯壳队列）。
///
/// **阶段边界即安全边界**：第 3 阶段的钩子会摘门闩、唤醒等待者，而此刻全部受害者
/// 都已停摆，没有「被唤醒后还能跑」的中间态。栈/名单都是局部 Vec（锁外分配），
/// 全程不持任何锁。
pub(crate) fn cull(roots: &[Arc<Team>], reason: usize) {
    let mut work: Vec<Arc<Team>> = roots.to_vec();
    let mut tasks: Vec<Arc<Task>> = Vec::new();
    while let Some(t) = work.pop() {
        // 先取本域成员快照（放锁），再收任务与子域；不持 team.tasks 锁。
        for weak_task in t.tasks_snapshot() {
            if let Some(task) = weak_task.upgrade() {
                work.extend(task.heirs());
                tasks.push(task);
            }
        }
    }
    let victims: Vec<Arc<Task>> = tasks.into_iter().filter(|t| suspend(t, reason)).collect();
    for task in victims {
        reap(task);
    }
}

/// 级联触发（挂 exit_hook）：读父 task 的 heir → 两阶段扑杀整棵血缘子树。
///
/// **只收子域**：同域线程之间没有寿命耦合——域亡＝成员清零（成员各自退场，都走完就
/// 没了），加上这里的子域级联。故"域里还留着一枚常驻线程"不是内核该管的事：谁起的
/// 谁收（会话的收尾由会话的主人负责，见 `programs/.../root/main.rs` 的 `board::shut`）。
pub(crate) fn doom(tid: usize) {
    if let Some(task) = muster(tid).and_then(|w| w.upgrade()) {
        // “谁杀的”由父域自己那一笔 `Exit` 记（它先于本行发出）——级联不为子树里每个
        // 任务各记一条 `Doomed`。
        cull(&task.heirs(), super::EXIT_CASCADE);
    }
}

/// 自退查询：**本核当前正在跑的那一枚**被判死了没有。在则摘出**原因码**交给调用方
/// （它写进本核的原因槽再 `quit`）。
///
/// 调用点是"任务自己的时刻"那几处（`SupervisorSoft` / `SupervisorTimer` / 离核前自查），
/// 故它是**取即清**：一笔杀令只兑现一次。表空时由 [`PENDING`] 当场挡掉——这条查询落在
/// 热路径上（每次离核 + 每个 tick），不该为它去碰 L3 锁。
pub(crate) fn take_doomed(tid: usize) -> Option<usize> {
    if PENDING.load(Ordering::Relaxed) == 0 {
        return None;
    }
    let out = doomed().lock().remove(&tid);
    if out.is_some() {
        PENDING.fetch_sub(1, Ordering::Relaxed);
    }
    out
}

/// 血缘判据：`actor` 是不是 `target` 的**祖先域**（含直接）。
///
/// 沿 `Team.sire`（弱引用、构造期定型）上溯，逐级用 `muster` 问名册；比较按**域**
/// ——`sire` 链上记的是**建域那一枚 task**，若按 task 比，同域的另一线程就杀不了
/// 自己的子域，而语义明明是域对域。
///
/// **严格祖先**：从目标域的 sire 起步（自己不算自己的后代）——否则"杀我域里的
/// 一枚线程"会退化成"杀掉我整个域连同我自己"。
///
/// 纯查询：不改任何状态；成本 O(深度)（virt 上 2）。锁纪律：逐级取放 `muster`
/// （L3），不跨级持锁；`Team::sire()` 只做一次 `Weak::upgrade`，不取锁。
///
/// # 今天没有调用者（`Doom` 已无血缘门，见 envcall 的 Ruin 口径）
///
/// 留着的理由：它是唯一一份"血缘判据"的实现，而这份判据**会回来**——编排侧要在自己的
/// 服务表之外做"谁能收谁"的裁决时（或 `disown` 落地之后），第一件要问的还是它。
/// 删掉它等于把这段推理一起删掉。
///
/// # 两个 `false` 不是同一件事
///
/// 本函数只答"是不是后代"，**答不出"为什么不是"**。故上溯途中把目标域那一端的链
/// 走断（父域已回收 ⇒ `Weak` 升不起来 / 名册里没它）与"走到了顶、actor 不在链上"
/// 同返 `false`，调用方（envcall 的 `Doom` 入口）只能拿它判 `-1 Denied`。而"目标
/// 自己不在世"是**另一条判据**（`muster` 找不到目标任务的条目 ⇒ `-2 Dead`），在进
/// 本函数之前就已经分出去了——**判活的码与判据的码不在这里合流**。"同域"与"顶级域"
/// 都走 `false` 这一支：前者被"严格"吃掉（从目标域的 `sire` 起步，域不是自己的
/// 后代），后者 `sire` 恒空。
#[allow(dead_code)] // 见上：判据会回来，留着它 = 留着这段推理
pub(crate) fn descends(actor: &Arc<Team>, target: &Arc<Team>) -> bool {
    let mut team = target.clone();
    loop {
        let Some(sire) = team.sire().and_then(muster) else {
            return false;
        };
        let Some(parent) = Weak::upgrade(&sire) else {
            return false;
        };
        if Arc::ptr_eq(&parent.ident.team, actor) {
            return true;
        }
        team = parent.ident.team.clone();
    }
}
