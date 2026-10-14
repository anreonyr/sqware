use alloc::sync::{Arc, Weak};

use crate::core::work::room::messenger::WakeKey;
use crate::core::work::unit::life::{Life, TeamLife};
use crate::core::work::unit::team::Member;

#[derive(Clone)]
pub(crate) enum JoinTarget {
    Task(Arc<Member>),
    Team(Arc<TeamLife>),
}

impl JoinTarget {
    pub(crate) fn key(&self) -> WakeKey {
        match self {
            Self::Task(member) => WakeKey::Task { id: member.id },
            Self::Team(team) => WakeKey::Team { id: team.id },
        }
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        match self {
            Self::Task(member) => Arc::downgrade(&member.life),
            Self::Team(team) => Arc::downgrade(&team.life),
        }
    }
}
