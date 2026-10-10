pub const INSTANCE_CAP: usize = 256;
use crate::system::control::unit::table::State;
use env::{TaskId, TeamId};
pub struct Instance {
    pub owner: TaskId,
    pub task: TaskId,
    pub team: Option<TeamId>,
    pub state: State,
    pub claimed: bool,
    pub started: bool,
    pub reason: Option<env::Reason>,
    pub claim_until: u64,
    pub hook: ::schedule::Cursor,
}

impl Instance {
    pub(crate) fn reap(&mut self, exit: env::TaskExit) {
        if exit.task == self.task {
            if exit.reason != 0 { self.reason = Some(exit.reason); }
            else { self.reason.get_or_insert(0); }
            self.stop();
        } else if exit.cause == env::ExitCause::Fault
            || exit.cause == env::ExitCause::Reap && exit.reason != 0
        {
            if self.reason.unwrap_or(0) == 0 { self.reason = Some(exit.reason); }
            self.stop();
        }
    }

    pub(crate) fn stop(&mut self) {
        if self.state != State::Stopping {
            self.hook.reset();
        }
        self.state = State::Stopping;
    }
}
