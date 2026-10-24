pub const INSTANCE_CAP: usize = 256;
use crate::system::control::unit::table::State;
use env::{TaskId, TeamId};
pub struct Instance {
    pub reference: crate::system::assembly::InstanceRef,
    pub owner: TaskId,
    pub task: TaskId,
    pub receipt: super::receipt::Receipt,
    pub state: State,
    pub claimed: bool,
    pub started: bool,
    pub reason: Option<env::Reason>,
    pub claim_until: u64,
}

impl Instance {
    pub(crate) fn settled(&self) -> bool {
        self.state == State::Dead && !matches!(self.receipt, super::receipt::Receipt::Returned(_))
    }

    pub(crate) fn team(&self) -> Option<TeamId> {
        (self.state != State::Dead).then(|| self.receipt.team()).flatten()
    }

    pub(crate) fn claim(&mut self, now: u64) -> Option<TeamId> {
        if self.claimed {
            return (matches!(self.state, State::Debarked | State::Ready)
                || self.state == State::Starting && self.started)
                .then_some(self.team())
                .flatten();
        }
        if now >= self.claim_until || self.state != State::Debarked {
            return None;
        }
        let team = self.team()?;
        self.claimed = true;
        Some(team)
    }

    pub(crate) fn reap(&mut self, exit: env::TaskExit) {
        if exit.task == self.task {
            if exit.reason != 0 {
                self.reason = Some(exit.reason);
            } else {
                self.reason.get_or_insert(0);
            }
            self.stop();
        } else if !self.claimed
            && (exit.cause == env::ExitCause::Fault
                || exit.cause == env::ExitCause::Reap && exit.reason != 0)
        {
            if self.reason.unwrap_or(0) == 0 {
                self.reason = Some(exit.reason);
            }
            self.stop();
        }
    }

    pub(crate) fn stop(&mut self) {
        if self.state != State::Dead {
            self.state = State::Stopping;
        }
    }
}
