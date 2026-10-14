use alloc::sync::Weak;

use env::{MailCondition, Permission, PieToken, Source, TaskId, UnitTarget};

use crate::core::work::unit::join::JoinTarget;
use crate::core::work::unit::task::Task;

#[derive(Clone)]
pub(crate) enum Watch {
    Mail {
        pie: PieToken,
        condition: MailCondition,
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
    pub(crate) fn source(&self) -> Source {
        match self {
            Watch::Mail { pie, condition } => Source::Mail {
                pie: *pie,
                condition: *condition,
            },
            Watch::Join { target, .. } => Source::Join { target: *target },
            Watch::Inspect { task, token, .. } => Source::Inspect {
                task: *task,
                token: *token,
            },
        }
    }
}
