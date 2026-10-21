use env::TaskId;
use system_api::identity::{FoundId, PrincipalId};

pub(super) struct CoalitionNode {
    pub(super) manager: PrincipalId,
    pub(super) caller: TaskId,
    pub(super) creation: FoundId,
}
