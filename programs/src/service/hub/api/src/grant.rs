//! Hub grant mapping tied to the generated provider declaration.

pub use super::interface::{Grant, grant_of};
use crate::frame::Wire;

impl Grant {
    pub const fn for_wire(wire: &Wire) -> Self {
        match wire {
            Wire::Bond(_) => Self::Bond,
            Wire::List(..) => Self::List,
            Wire::Claim { .. } => Self::Claim,
        }
    }

    pub const fn of_wire(wire: &Wire) -> u8 {
        Self::for_wire(wire).at()
    }
}
