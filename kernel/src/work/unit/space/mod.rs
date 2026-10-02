mod access;
mod inner;
mod map;
mod outer;
mod salvage;
mod segment;
pub(crate) mod window;

pub(crate) use map::{Pending, PendingState};
pub use outer::{Space, SpaceBuilder};
pub(crate) use salvage::Span;
pub(crate) use segment::SegmentKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    Supervisor,
    User,
}

impl SpaceKind {
    pub fn is_supervisor(self) -> bool {
        matches!(self, SpaceKind::Supervisor)
    }
}

impl From<env::ProgramKind> for SpaceKind {
    fn from(k: env::ProgramKind) -> Self {
        match k {
            env::ProgramKind::User => SpaceKind::User,
            env::ProgramKind::Supervisor => SpaceKind::Supervisor,
        }
    }
}
