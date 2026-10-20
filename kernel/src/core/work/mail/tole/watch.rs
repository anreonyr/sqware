use alloc::sync::Weak;

use env::{Permission, PieToken, TaskId, ToleEventSource, UnitTarget};

use crate::core::work::unit::join::JoinTarget;
use crate::core::work::unit::task::Task;

#[derive(Clone)]
pub(crate) enum Watch {
    Resource {
        source: ToleEventSource,
    },
    Join {
        target: UnitTarget,
        selected: JoinTarget,
    },
    Inspect {
        task: TaskId,
        token: PieToken,
        holder: Weak<Task>,
        parent: Option<(Weak<Task>, PieToken)>,
        held: bool,
        access: Permission,
    },
}

impl Watch {
    pub(crate) fn source(&self) -> ToleEventSource {
        match self {
            Watch::Resource { source } => *source,
            Watch::Join { target, .. } => ToleEventSource::Join { target: *target },
            Watch::Inspect { task, token, .. } => ToleEventSource::Inspect {
                task: *task,
                token: *token,
            },
        }
    }
}
