use hub_api::DeviceId;

use super::{Enrollment, Entry, Owner};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cell {
    Enrolling(Enrollment),
    Vacant { entry: Entry, origin: Option<DeviceId> },
    Held { entry: Entry, origin: Option<DeviceId>, owner: Owner },
}

impl Cell {
    pub fn entry(&self) -> &Entry {
        match self {
            Self::Enrolling(enrollment) => &enrollment.entry,
            Self::Vacant { entry, .. } | Self::Held { entry, .. } => entry,
        }
    }

    pub fn device(&self) -> Option<DeviceId> {
        match self {
            Self::Enrolling(enrollment) => enrollment.origin,
            Self::Vacant { origin, .. } | Self::Held { origin, .. } => *origin,
        }
    }

    pub fn ready(&self) -> bool {
        !matches!(self, Self::Enrolling(_))
    }

    pub fn held(&self) -> bool {
        matches!(self, Self::Held { .. })
    }
}
