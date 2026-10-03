use crate::system::control::core::unit::{Slot, State, Table};
use crate::system::identity::serve::install::Roster;
use crate::system::identity::serve::query::{binding, current_authority, validate_permit};
use crate::system::operator::serve::install::Tree;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use env::{TaskId, TeamId, Wait};
use protocol::common::path::{Path, PathBuf};
use protocol::system::identity::Selector;
use protocol::system::operator::{EntryId, Fail, Permit};
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
    pub fn prepare(
        &mut self,
        table: &Table,
        roster: &Roster,
        tree: &mut Tree,
    ) -> Result<(), &'static str> {
        if current_authority(roster).is_none() || tree.host().is_none() {
            return Ok(());
        }
        for row in table.living() {
            let Slot::Live {
                task,
                team: Some(team),
            } = row.slot
            else {
                continue;
            };
            if self.runs.iter().any(|r| r.task == task)
                || !matches!(
                    row.state,
                    State::NeverStarted | State::Starting | State::Ready
                )
                || !live(table, task)
            {
                continue;
            }
            let Some(_) = binding(roster, task).map_err(|_| "runtime identity query")? else {
                continue;
            };
            self.runs.try_reserve(1).map_err(|_| "runtime capacity")?;
            let team_road = Path::new("uit")
                .try_join(&team.get().to_string())
                .ok_or("runtime team path")?;
            let road = team_road
                .try_join(&task.get().to_string())
                .ok_or("runtime task path")?;
            let team_pane = tree.mount(&team_road, None, Permit::Public, None, false)?;
            let pane = match tree.mount(&road, None, Permit::Public, None, false) {
                Ok(pane) => pane,
                Err(why) => {
                    let _ = tree.unmount(team_pane);
                    return Err(why);
                }
            };
            self.runs.push(Run {
                task,
                team,
                road,
                pane,
                team_pane,
                kinds: Vec::new(),
            });
        }
        Ok(())
    }
    pub fn policy(
        &self,
        table: &Table,
        roster: &Roster,
        from: TaskId,
        task: TaskId,
        kind: &str,
        name: &str,
        requested: Permit,
    ) -> Result<(PathBuf, Permit, TaskId), Fail> {
        let run = self
            .runs
            .iter()
            .find(|r| r.task == task)
            .ok_or(Fail::Denied)?;
        if !live(table, task) {
            return Err(Fail::Denied);
        }
        let permit = if let Some(approval) = self
            .approvals
            .iter()
            .find(|a| a.service == from && a.task == task && a.kind == kind && a.name == name)
        {
            approval.permit
        } else if from == task {
            match kind {
                "public" => Permit::Public,
                "hole" => Permit::Identity(Selector::Exact(
                    binding(roster, from)?
                        .ok_or(Fail::Denied)?
                        .current
                        .principal,
                )),
                "bound" => Permit::Bound,
                _ => return Err(Fail::Denied),
            }
        } else {
            return Err(Fail::Denied);
        };
        if permit != requested {
            return Err(Fail::Denied);
        }
        validate_permit(roster, permit)?;
        Ok((
            run.road
                .try_join(kind)
                .and_then(|p| p.try_join(name))
                .ok_or(Fail::Denied)?,
            permit,
            task,
        ))
    }
    pub fn remove(
        &mut self,
        tree: &mut Tree,
        live: impl Fn(TaskId) -> bool,
    ) -> Result<(), &'static str> {
        let mut at = 0;
        while at < self.runs.len() {
            if live(self.runs[at].task) {
                at += 1;
                continue;
            }
            let r = &self.runs[at];
            // Only this runtime's empty children may be removed, deepest first.
            for road in r.kinds.iter().rev() {
                tree.remove_empty(road)?;
            }
            tree.unmount(r.pane)?;
            if !self
                .runs
                .iter()
                .any(|other| other.task != r.task && other.team == r.team)
            {
                tree.unmount(r.team_pane)?;
            }
            self.approvals
                .retain(|a| a.task != r.task && a.service != r.task);
            self.runs.remove(at);
        }
        self.approvals.retain(|a| live(a.service) && live(a.task));
        Ok(())
    }
}
fn live(table: &Table, task: TaskId) -> bool {
    if task == runtime::env::unit::self_id() {
        return true;
    }
    table.living().any(|row| {
        matches!(row.slot, Slot::Live { task: known, .. } if known == task)
            && matches!(
                row.state,
                State::NeverStarted | State::Starting | State::Ready
            )
            && !runtime::env::unit::join(task, Wait::POLL).unwrap_or(true)
    })
}
