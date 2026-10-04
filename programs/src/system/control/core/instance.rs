pub const INSTANCE_CAP: usize = 256;
use super::unit::State;
use env::{TaskId, TeamId};
pub struct Instance {
    pub owner: TaskId,
    pub task: TaskId,
    pub team: Option<TeamId>,
    pub state: State,
    pub claimed: bool,
    pub claim_until: u64,
}
