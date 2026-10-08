//! Operator grant mapping tied to the generated provider declaration.

use super::frame::Wire;
pub use super::interface::{Grant, grant_of};

impl Grant {
    pub const fn for_wire(wire: &Wire) -> Self {
        match wire {
            Wire::Part { .. } => Self::Part,
            Wire::Land { .. } => Self::Land,
            Wire::Find(_) => Self::Find,
            Wire::Trim(_) => Self::Trim,
            Wire::List(_) => Self::List,
            Wire::Road(_) => Self::Seek,
            Wire::Name(_) => Self::Name,
            Wire::Watch { .. } => Self::Watch,
        }
    }

    pub const fn of_wire(wire: &Wire) -> u8 {
        Self::for_wire(wire).at()
    }
}
