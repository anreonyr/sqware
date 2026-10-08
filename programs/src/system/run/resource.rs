use super::living::Living;
use crate::system::control::core::unit::{Slot, State};
use crate::system::control::identity::Roster;
use crate::system::control::identity::{binding, current_authority};
use crate::system::control::unit::Control;
use crate::system::operator::Placement;
use crate::system::operator::client::Tree;
use crate::system::operator::core::Tile;
use ::schedule::{Progress, Res, ResMut};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use env::{TaskId, TeamId};
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Permit;
use system_api::operator::path::Path;
use system_api::operator::path::PathBuf;
struct Run {
    task: TaskId,
    team: TeamId,
    road: PathBuf,
    pane: EntryId,
    team_pane: EntryId,
    kinds: Vec<PathBuf>,
}
pub struct Approval {
    pub service: TaskId,
    pub task: TaskId,
    pub kind: String,
    pub name: String,
    pub permit: Permit,
}
pub struct Resources {
    runs: Vec<Run>,
    approvals: Vec<Approval>,
}
impl Resources {
    pub(crate) fn team(&self, task: TaskId) -> Option<TeamId> {
        self.runs.iter().find(|r| r.task == task).map(|r| r.team)
    }
    pub(crate) fn prepare_kind(
        &mut self,
        task: TaskId,
        kind: &str,
    ) -> Result<Option<PathBuf>, Fail> {
        let run = self
            .runs
            .iter_mut()
            .find(|r| r.task == task)
            .ok_or(Fail::Denied)?;
        let road = run.road.try_join(kind).ok_or(Fail::Denied)?;
        if run.kinds.contains(&road) {
            return Ok(None);
        }
        run.kinds.try_reserve(1).map_err(|_| Fail::Full)?;
        Ok(Some(road))
    }
    pub(crate) fn commit_kind(&mut self, task: TaskId, road: PathBuf) -> Result<(), Fail> {
        self.runs
            .iter_mut()
            .find(|r| r.task == task)
            .ok_or(Fail::Denied)?
            .kinds
            .push(road);
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            approvals: Vec::new(),
        }
    }
    pub(crate) fn runtime_road(&self, task: TaskId) -> Option<PathBuf> {
        self.runs
            .iter()
            .find(|r| r.task == task)
            .map(|r| r.road.clone())
    }
    pub fn approve(&mut self, approval: Approval) -> Result<(), &'static str> {
        self.approvals
            .try_reserve(1)
            .map_err(|_| "resource policy capacity")?;
        self.approvals.push(approval);
        Ok(())
    }

    pub(crate) fn policy(
        &self,
        incoming: &crate::system::run::publication::Incoming,
    ) -> Result<crate::system::run::publication::Decision, Fail> {
        use crate::system::run::publication::{Approved, Decision};
        use system_api::control::publication::Target;
        let target = incoming.frame.target().ok_or(Fail::Denied)?;
        let Target::RuntimeResource { task, kind, name } = &target else {
            return Err(Fail::Denied);
        };
        let run = self
            .runs
            .iter()
            .find(|r| r.task == *task)
            .ok_or(Fail::Denied)?;
        let approval = self.approvals.iter().find(|a| {
            a.service == incoming.from && a.task == *task && a.kind == *kind && a.name == *name
        });
        let own_hole = approval.is_none() && incoming.from == *task && kind == "hole";
        let permit = if let Some(approval) = approval {
            approval.permit
        } else if incoming.from == *task {
            match kind.as_str() {
                "public" => Permit::Public,
                "bound" => Permit::Bound,
                "hole" => incoming.frame.permit,
                _ => return Err(Fail::Denied),
            }
        } else {
            return Err(Fail::Denied);
        };
        if permit != incoming.frame.permit {
            return Err(Fail::Denied);
        }
        let road = run
            .road
            .try_join(kind)
            .and_then(|p| p.try_join(name))
            .ok_or(Fail::Denied)?;
        let approved = Approved {
            target: target.clone(),
            placement: Placement {
                road,
                tile: Tile {
                    pie: incoming.frame.entry,
                    permit,
                    owner: Some(*task),
                },
                replace: false,
            },
            publisher: incoming.from,
        };
        Ok(if own_hole {
            Decision::OwnHole(approved)
        } else {
            Decision::Install(approved)
        })
    }
}
pub(crate) fn retire(
    living: Res<Living>,
    mut resources: ResMut<Resources>,
    mut tree: ResMut<Tree>,
) -> Result<Progress, &'static str> {
    let live = |task| living.contains(task);
    let resources = &mut *resources;

    let mut at = 0;
    while at < resources.runs.len() {
        if live(resources.runs[at].task) {
            at += 1;
            continue;
        }
        let r = &resources.runs[at];
        // Only this runtime's empty children may be removed, deepest first.
        for road in r.kinds.iter().rev() {
            tree.remove_empty(road)?;
        }
        tree.unmount(r.pane)?;
        if !resources
            .runs
            .iter()
            .any(|other| other.task != r.task && other.team == r.team)
        {
            tree.unmount(r.team_pane)?;
        }
        resources
            .approvals
            .retain(|a| a.task != r.task && a.service != r.task);
        resources.runs.remove(at);
    }
    resources
        .approvals
        .retain(|a| live(a.service) && live(a.task));
    Ok(Progress::Done)
}
pub(crate) fn candidates(
    control: Res<Control>,
    resources: Res<Resources>,
    mut pending: ResMut<Runtimes>,
) -> Result<Progress, &'static str> {
    pending.requests.clear();
    for row in control.living() {
        let Slot::Live {
            task,
            team: Some(team),
        } = row.slot
        else {
            continue;
        };
        if resources.runs.iter().any(|run| run.task == task)
            || !matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready | State::Debarked
            )
            || !control.live_service(task)
        {
            continue;
        }
        pending
            .requests
            .try_reserve(1)
            .map_err(|_| "runtime capacity")?;
        pending.requests.push((task, team));
    }
    for item in control.instances() {
        let Some(team) = item.team else {
            continue;
        };
        if !control.live(item.task) || resources.runs.iter().any(|run| run.task == item.task) {
            continue;
        }
        pending
            .requests
            .try_reserve(1)
            .map_err(|_| "runtime capacity")?;
        pending.requests.push((item.task, team));
    }
    Ok(Progress::Done)
}
pub(crate) fn prepare(
    roster: Res<Roster>,
    epoch: Res<crate::system::identity::revision::Epoch>,
    mut pending: ResMut<Runtimes>,
) -> Result<Progress, &'static str> {
    if pending.requests.is_empty() {
        return Ok(Progress::Done);
    }
    let revision = epoch.0.load(core::sync::atomic::Ordering::Acquire);
    if pending.seen != revision {
        pending.seen = revision;
        pending.checked.clear();
    } else {
        let mut at = 0;
        while at < pending.requests.len() {
            if pending.checked.contains(&pending.requests[at].0) {
                pending.requests.remove(at);
            } else {
                at += 1;
            }
        }
    }
    if current_authority(&roster).is_none() {
        pending.requests.clear();
        return Ok(Progress::Done);
    }
    let count = pending.requests.len();
    pending
        .checked
        .try_reserve(count)
        .map_err(|_| "runtime identity capacity")?;
    let mut at = 0;
    while at < pending.requests.len() {
        if binding(&roster, pending.requests[at].0)
            .map_err(|_| "runtime identity query")?
            .is_some()
        {
            at += 1;
        } else {
            let (task, _) = pending.requests.remove(at);
            pending.checked.push(task);
        }
    }
    Ok(Progress::Done)
}
pub struct Runtimes {
    pub requests: Vec<(TaskId, TeamId)>,
    pub seen: u64,
    pub checked: Vec<TaskId>,
}
pub(crate) fn install(
    mut pending: ResMut<Runtimes>,
    mut tree: ResMut<Tree>,
    mut resources: ResMut<Resources>,
) -> Result<Progress, &'static str> {
    if tree.host().is_none() {
        pending.requests.clear();
        return Ok(Progress::Done);
    }
    for (task, team) in pending.requests.drain(..) {
        if resources.runs.iter().any(|r| r.task == task) {
            continue;
        }
        resources
            .runs
            .try_reserve(1)
            .map_err(|_| "runtime capacity")?;
        let team_road = Path::new("uit")
            .try_join(&team.get().to_string())
            .ok_or("runtime team path")?;
        let road = team_road
            .try_join(&task.get().to_string())
            .ok_or("runtime task path")?;
        let team_pane = tree.mount(&Placement {
            road: (&team_road).to_path_buf(),
            tile: Tile {
                pie: env::PieToken::NONE,
                permit: Permit::Public,
                owner: None,
            },
            replace: false,
        })?;
        let pane = match tree.mount(&Placement {
            road: (&road).to_path_buf(),
            tile: Tile {
                pie: env::PieToken::NONE,
                permit: Permit::Public,
                owner: None,
            },
            replace: false,
        }) {
            Ok(pane) => pane,
            Err(why) => {
                let _ = tree.unmount(team_pane);
                return Err(why);
            }
        };
        resources.runs.push(Run {
            task,
            team,
            road,
            pane,
            team_pane,
            kinds: Vec::new(),
        });
    }
    Ok(Progress::Done)
}
