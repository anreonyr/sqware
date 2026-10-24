use self::state::Instance;
use self::receipt::Receipt;
use super::unit::table::State;
use crate::system::app::Fault as ControlFail;
use crate::system::control::unit::Control;

use env::{TaskId, Wait, unit};
use system_api::{control::Fail, loader::Built};

impl Control {
    pub(crate) fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.iter()
    }

    pub(crate) fn instance(&self, task: TaskId) -> Option<&Instance> {
        self.instances.iter().find(|item| item.task == task)
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
            State::Debarked => match item.team() {
                Some(team) => Ok(Some(Ok(Built {
                    task: item.task,
                    team,
                }))),
                None => Err(ControlFail::Room),
            },
            _ => Ok(Some(Err(Fail::NotReady))),
        }
    }

    pub(crate) fn claim_instance(&mut self, owner: TaskId, task: TaskId) -> Option<env::TeamId> {
        let item = self
            .instances
            .iter_mut()
            .find(|item| item.task == task && item.owner == owner)?;
        item.claim(env::chrono::clock())
    }

    pub(crate) fn stop_instance(&mut self, task: TaskId) -> bool {
        let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.task == task)
        else {
            return false;
        };
        instance.stop();
        if let Some(team) = instance.team() {
            let _ = unit::slay_team(team);
        }
        true
    }

    pub(crate) fn activate_instance(
        &mut self,
        reference: crate::system::assembly::InstanceRef,
    ) -> bool {
        let Some(item) = self
            .instances
            .iter_mut()
            .find(|item| item.reference == reference)
        else {
            return false;
        };
        if item.task != reference.task || item.state != State::Starting || !item.started {
            return false;
        }
        item.state = State::Ready;
        true
    }

    pub(crate) fn fail_activation(
        &mut self,
        reference: crate::system::assembly::InstanceRef,
    ) -> bool {
        let Some(item) = self
            .instances
            .iter_mut()
            .find(|item| item.reference == reference)
        else {
            return false;
        };
        if item.task != reference.task || item.state != State::Starting || !item.started {
            return false;
        }
        item.stop();
        if let Some(team) = item.team() {
            let _ = unit::slay_team(team);
        }
        true
    }

    pub(crate) fn reap_instances(&mut self, settling: bool) {
        self.reclaim_returned();
        for item in &mut self.instances {
            if item.state == State::Dead {
                continue;
            }
            if item.team().is_none() {
                continue;
            }
            if item.claimed {
                if let Ok(env::JoinReply::Reaped(exit)) =
                    unit::join(item.task.into(), Wait::POLL, false)
                {
                    item.reap(exit);
                }
            } else if let Some(team) = item.team() {
                // Receive every member independently; main/auxiliary policy is Control's.
                for _ in 0..64 {
                    match unit::join(env::UnitTarget::Team(team), Wait::POLL, true) {
                        Ok(env::JoinReply::Reaped(exit)) => item.reap(exit),
                        _ => break,
                    }
                }
            }
            if settling
                || (!item.claimed
                    && item.state == State::Debarked
                    && env::chrono::clock() >= item.claim_until)
                || unit::join_task(item.owner, Wait::POLL).unwrap_or(true)
                || unit::join_task(item.task, Wait::POLL).unwrap_or(true)
            {
                item.stop();
            }
            if item.state == State::Stopping {
                if let Some(team) = item.team() {
                    let _ = unit::slay_team(team);
                }
            }
        }
        self.instances.retain(|item| item.retains_after_reap());
    }

    pub(crate) fn instance_wait(&self, mut wait: Wait) -> Wait {
        for item in self
            .instances
            .iter()
            .filter(|item| !item.claimed && item.team().is_some() && item.state == State::Debarked)
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
            .any(|item| matches!(item.state, State::Starting | State::Stopping)
                || matches!(item.receipt, Receipt::Returned(_)))
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

pub(crate) mod activation;
pub(crate) mod create;
pub(crate) mod hook;

mod command;
pub(crate) use command::Command;

pub(crate) mod schedule;
pub(crate) mod state;
pub(crate) mod receipt;
mod release;
