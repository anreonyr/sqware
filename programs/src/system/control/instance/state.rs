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
    pub(crate) fn stop(&mut self) {
        if self.state != State::Stopping {
            self.hook.reset();
        }
        self.state = State::Stopping;
    }
}
