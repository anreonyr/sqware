use super::{Enrollment, Entry, Owner};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cell {
    Enrolling(Enrollment),
    Vacant(Entry),
    Held { entry: Entry, owner: Owner },
}

impl Cell {
    pub fn entry(&self) -> &Entry {
        match self {
            Self::Enrolling(enrollment) => &enrollment.entry,
            Self::Vacant(entry) | Self::Held { entry, .. } => entry,
        }
    }

    pub fn ready(&self) -> bool {
        !matches!(self, Self::Enrolling(_))
    }

    pub fn held(&self) -> bool {
        matches!(self, Self::Held { .. })
    }
}
