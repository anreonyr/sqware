use crate::system::control::core::{
    instance::{self, Instance},
    unit::State,
};
use crate::system::control::{Fail as ControlFail, unit::Control};
use env::{TaskId, Wait, unit};
use system_api::{control::Fail, loader::Built};

impl Control {
    pub(crate) fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter()
    }

    pub(crate) fn instance(&self, task: TaskId) -> Option<&Instance> {
        self.instances.iter().find(|item| item.task == task)
    }

    pub(crate) fn owns_team_instance(&self, owner: TaskId) -> bool {
        self.instances
            .iter()
            .any(|item| item.owner == owner && item.team.is_some())
    }

    pub(crate) fn instance_result(
        &self,
        task: TaskId,
    ) -> Result<Option<Result<Built, Fail>>, ControlFail> {
        let Some(item) = self.instance(task) else {
            return Ok(Some(Err(Fail::NotReady)));
        };
        match item.state {
            State::Starting => Ok(None),
            State::Debarked => match item.team {
                Some(team) => Ok(Some(Ok(Built {
                    task: item.task,
                    team,
                }))),
                None => Err(ControlFail::Room),
            },
            _ => Ok(Some(Err(Fail::NotReady))),
        }
    }

    pub(super) fn reserve_instance(&mut self) -> Result<(), Fail> {
        if self.instances.len() >= instance::INSTANCE_CAP {
            if let Some(at) = self
                .instances
                .iter()
                .position(|item| item.state == State::Dead)
            {
                self.instances.remove(at);
            } else {
                return Err(Fail::Full);
            }
        }
        self.instances.try_reserve(1).map_err(|_| Fail::Full)
    }

    pub(super) fn register_instance(&mut self, built: Built, owner: TaskId) {
        self.instances.push(Instance {
            owner,
            task: built.task,
            team: Some(built.team),
            state: State::Starting,
            claimed: false,
            claim_until: env::chrono::clock() + system_api::loader::CLAIM_MS as u64 * 1_000_000,
            hook: Default::default(),
        });
    }

    pub(crate) fn stop_instance(&mut self, task: TaskId) {
        if let Some(item) = self
            .instances
            .iter_mut()
            .find(|item| item.task == task && item.team.is_some())
        {
            item.stop();
        }
    }

    pub(crate) fn claim_instance(&mut self, owner: TaskId, task: TaskId) -> Option<env::TeamId> {
        let item = self
            .instances
            .iter_mut()
            .find(|item| item.task == task && item.owner == owner)?;
        if env::chrono::clock() >= item.claim_until || item.state != State::Debarked {
            return None;
        }
        let team = item.team?;
        item.claimed = true;
        Some(team)
    }

    pub(crate) fn reap_instances(&mut self, settling: bool) {
        for item in &mut self.instances {
            if item.team.is_none() {
                continue;
            }
            if settling
                || (!item.claimed && env::chrono::clock() >= item.claim_until)
                || unit::join(item.owner, Wait::POLL).unwrap_or(true)
                || unit::join(item.task, Wait::POLL).unwrap_or(true)
            {
                item.stop();
            }
            if item.state == State::Stopping {
                let _ = env::room::doom(item.task);
            }
        }
        self.instances.retain(|item| {
            item.team.is_some() || !unit::join(item.owner, Wait::POLL).unwrap_or(true)
        });
    }

    pub(crate) fn instance_wait(&self, mut wait: Wait) -> Wait {
        for item in self
            .instances
            .iter()
            .filter(|item| !item.claimed && item.team.is_some())
        {
            let ms = item
                .claim_until
                .saturating_sub(env::chrono::clock())
                .div_ceil(1_000_000)
                .max(1) as usize;
            wait = min_wait(wait, Wait::AtMost(ms));
        }
        if self
            .instances
            .iter()
            .any(|item| matches!(item.state, State::Starting | State::Stopping))
        {
            wait = Wait::AtMost(1);
        }
        wait
    }
}

fn min_wait(a: Wait, b: Wait) -> Wait {
    match (a, b) {
        (Wait::AtMost(a), Wait::AtMost(b)) => Wait::AtMost(a.min(b)),
        (Wait::AtMost(a), _) | (_, Wait::AtMost(a)) => Wait::AtMost(a),
        (Wait::Forever, Wait::Forever) => Wait::Forever,
    }
}

pub(crate) mod create;
pub(crate) mod hook;

mod command;
pub(crate) use command::Command;
