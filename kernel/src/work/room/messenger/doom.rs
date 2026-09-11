// 扑杀（doom）——血缘级联的「杀」侧，**两阶段**：先停摆（摘出全部调度/等待
// 容器），再收尾（[`super::reap`]：钩子 → Reaped → 入躯壳队列）。
//
// 两阶段是**正确性要求**，不是优化：钩子会摘门闩，摘门闩会唤醒等待者；若受害者
// 尚未停摆，它可能被别的核偷走并运行，在「已注定要死」的状态下观察到一个已死的
// 资源。他核 Running 任务无法被本核同步拉走（会破坏「Reaped 不在 running 槽」
// 不变量），故走 `doomed` 待杀集合 + SSIP 单点，目标核 trap 自查自退——最终一致。

use alloc::sync::Arc;
use alloc::vec::Vec;

use hashbrown::HashSet;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::room::scheduler::core::muster;
use crate::work::unit::task::{Task, TaskState, TaskTag};
use crate::work::unit::team::Team;

use super::reap::reap;
use super::wait::holder::Ticket;
use super::wait::site::{SITE_SHARDS, WakeKey, shard_at};
use super::{prune, void};

/// 待杀集合（doomed）：`kill` 点名他核 Running 任务时记入，目标核 trap 自查
/// 自退。无主簿记——只存 task_id，不持 `Arc<Task>`（防「杀者撑着被杀者」）。
/// Level::L3，与站点表同级。
pub(super) fn doomed() -> &'static SpinLock<HashSet<usize>> {
    static T: OnceLock<SpinLock<HashSet<usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashSet::new()))
}

/// 停摆单线程：摘出全部调度/等待容器 → 置 `Doomed`。返 `true` = 本次停摆了它，
/// 调用方须随后 [`reap`]；`false` = 没动它（已 `Doomed`/`Reaped`、不在任何容器，
/// 或 `Running`——后者已记 doomed + 定向 IPI，待其自退时自己 `reap`）。
///
/// **观察者纪律**（B′）：本函数读的是**判别式**（[`Task::tag`]，原子），payload 在类型
/// 上够不着——`Blocked` 的键与票因此不是读出来的，而是**问站点表**（扫分片找这个 task
/// 的等待者，顺带拿到 key + ticket）。tag 只是**提示**，容器操作的返回值才是**结论**：
/// 两者不一致（窗口内被别核 `seat` 走 / 被 `wipe` 放行）就重来一轮，重试耗尽按
/// `Running` 兜底（记 doomed + 定向 IPI）——**保证「要么当场摘掉、要么注定自退」**，
/// 不再有「读到 Starved 但摘不到 ⇒ 静默丢掉这次 kill」。
///
/// 锁纪律：只持 L3 表，逐表取、放锁后再取下一表（L3 同层绝不嵌套）；锁内不 drop
/// Arc（drop 链触 L2）。
fn suspend(task: &Arc<Task>) -> bool {
    /// 窗口有多小都可能有：给几次重来，之后走兜底。
    const RETRY: usize = 4;

    for _ in 0..=RETRY {
        let taken = match task.tag() {
            TaskTag::Reaped | TaskTag::Doomed => return false,
            TaskTag::Held => {
                // 未放行的引导线程：从 Team.held 摘出（不是它则放回）。
                let team = task.ident.team.clone();
                match team.take_held() {
                    Some(held) if Arc::ptr_eq(&held, task) => true,
                    Some(held) => {
                        team.hold(&held);
                        false
                    }
                    None => false,
                }
            }
            TaskTag::Starved => crate::work::room::scheduler::core::remove_from_starved(task),
            TaskTag::Blocked => {
                // 「读状态再摘容器」做不出来了 ⇒ 问容器：扫分片找它的等待者。
                let Some(ticket) = pop_waiter(task) else {
                    continue;
                };
                // 作废票根 + 消音到点（[`void`] 幂等）。
                void(ticket);
                true
            }
            TaskTag::Running => return doomed_nudge(task),
        };
        if taken {
            let mut t = task.clone();
            Task::exclusive(&mut t).transform(TaskState::Doomed);
            return true;
        }
        // 容器里没有它（tag 陈旧）：重来一轮重新分派。
    }
    // 重试耗尽：tag 一直在变（正在换容器）⇒ 按 Running 兜底。
    doomed_nudge(task)
}

/// 站点表全扫：找 `task` 的等待者，摘出并**顺带拿到它的票**（观察者不读 payload，
/// 键票只能问容器——键在片内用来定位与 `prune`，票交调用方作废到点登记）。
/// 逐片取放，不持跨片锁；命中即摘即返。
fn pop_waiter(task: &Arc<Task>) -> Option<Ticket> {
    for shard in 0..SITE_SHARDS {
        let mut sites = shard_at(shard).lock();
        let mut hit: Option<(WakeKey, Ticket)> = None;
        for (key, site) in sites.iter_mut() {
            if let Some(i) = site.waiters.iter().position(|w| Arc::ptr_eq(&w.task, task)) {
                let w = site.waiters.remove(i).expect("idx from position");
                hit = Some((*key, w.ticket));
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
/// 自查自退。找不到它的在跑核也照样记——**「记了 doomed」本身就是保证**：它下次上台
/// 遇任何 IPI 即自退；不记才会把这次 kill 丢掉。
fn doomed_nudge(task: &Arc<Task>) -> bool {
    doomed().lock().insert(task.ident.id);
    if let Some(hart) = crate::work::room::scheduler::core::running_hart(task) {
        crate::work::room::conductor::nudge(hart);
    }
    false
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
pub(crate) fn cull(roots: &[Arc<Team>]) {
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
    let victims: Vec<Arc<Task>> = tasks.into_iter().filter(suspend).collect();
    for task in victims {
        reap(task);
    }
}

/// 级联触发（挂 exit_hook）：读父 task 的 heir → 两阶段扑杀整棵血缘子树。
pub(crate) fn doom(tid: usize) {
    if let Some(task) = muster(tid).and_then(|w| w.upgrade()) {
        cull(&task.heirs());
    }
}

/// trap(SupervisorSoft) 自退查询：本 hart 当前 running 任务是否被判死。
/// 在则摘出待杀标记并返回 true（调用方 quit）；否则 false。
pub(crate) fn take_doomed(tid: usize) -> bool {
    doomed().lock().remove(&tid)
}
