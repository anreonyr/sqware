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
use crate::work::unit::task::{Task, TaskState};
use crate::work::unit::team::Team;

use super::reap::reap;
use super::{prune, sites, void};

/// 待杀集合（doomed）：`kill` 点名他核 Running 任务时记入，目标核 trap 自查
/// 自退。无主簿记——只存 task_id，不持 `Arc<Task>`（防「杀者撑着被杀者」）。
/// Level::L3，与站点表同级。
pub(super) fn doomed() -> &'static SpinLock<HashSet<usize>> {
    static T: OnceLock<SpinLock<HashSet<usize>>> = OnceLock::new();
    T.get_or_init(|| SpinLock::new_level(Level::L3, HashSet::new()))
}

/// 停摆单线程：摘出全部调度/等待容器 → 置 `Doomed`。返 `true` = 本次停摆了它，
/// 调用方须随后 [`reap`]；`false` = 没动它（已 `Doomed`/`Reaped`、不在任何容器，
/// 或 `Running`——后者已记 doomed + SSIP，待其自退时自己 `reap`）。
///
/// 锁纪律：只持 L3 表，逐表取、放锁后再取下一表（L3 同层绝不嵌套）；锁内不 drop
/// Arc（drop 链触 L2）。
fn suspend(task: &Arc<Task>) -> bool {
    let taken = match task.state() {
        TaskState::Reaped | TaskState::Doomed => false,
        TaskState::Held => {
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
        TaskState::Starved => crate::work::room::scheduler::core::remove_from_starved(task),
        TaskState::Blocked { key, ticket } => {
            // 读票直达：键指出容器、票号指出队列里的哪一个——不必扫分片。
            let popped = {
                let mut sites = sites(key).lock();
                let w = sites.get_mut(&key).and_then(|site| {
                    site.waiters
                        .iter()
                        .position(|w| w.ticket == ticket)
                        .map(|i| site.waiters.remove(i).expect("idx from position"))
                });
                prune(&mut sites, key);
                w
            };
            if popped.is_none() {
                return false;
            }
            // 作废票根 + 消音到点（[`void`] 幂等）。
            void(ticket);
            true
        }
        TaskState::Running { .. } => {
            if let Some(hart) = crate::work::room::scheduler::core::running_hart(task) {
                doomed().lock().insert(task.ident.id);
                crate::work::room::conductor::nudge(hart);
            }
            false
        }
    };
    if taken {
        let mut t = task.clone();
        Task::exclusive(&mut t).transform(TaskState::Doomed);
    }
    taken
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
    let victims: Vec<Arc<Task>> = tasks.into_iter().filter(|t| suspend(t)).collect();
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
