//! Identity grant behavior tied to the generated provider declaration.

use super::frame::Wire;
pub use super::interface::{Grant, grant_of};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Public,
    Bound,
    Installer,
}

impl Grant {
    pub const fn for_wire(wire: &Wire) -> Self {
        match wire {
            Wire::Resolve(_) => Self::Resolve,
            Wire::Matches(..) => Self::Matches,
            Wire::Same(..) => Self::Same,
            Wire::Sire(_) => Self::Sire,
            Wire::Heir(..) => Self::Heir,
            Wire::Amid(..) => Self::Amid,
            Wire::Members(..) => Self::Members,
            Wire::Memberships(..) => Self::Memberships,
            Wire::Adopt(_) => Self::Adopt,
            Wire::Waive => Self::Waive,
            Wire::Restrict(_) => Self::Restrict,
            Wire::Derive(_) => Self::Derive,
            Wire::Found => Self::Found,
            Wire::Admit(..) => Self::Admit,
            Wire::Expel(..) => Self::Expel,
            Wire::Bind(..) => Self::Bind,
            Wire::Unbind(_) => Self::Unbind,
            Wire::Activate(..) => Self::Activate,
        }
    }

    pub const fn of_wire(wire: &Wire) -> u8 {
        Self::for_wire(wire).at()
    }

    pub const fn mount(self) -> Mount {
        match self {
            Self::Resolve
            | Self::Matches
            | Self::Same
            | Self::Sire
            | Self::Heir
            | Self::Amid
            | Self::Members
            | Self::Memberships => Mount::Public,
            Self::Adopt
            | Self::Waive
            | Self::Restrict
            | Self::Derive
            | Self::Found
            | Self::Admit
            | Self::Expel
            | Self::Activate => Mount::Bound,
            Self::Bind | Self::Unbind => Mount::Installer,
        }
    }
}
