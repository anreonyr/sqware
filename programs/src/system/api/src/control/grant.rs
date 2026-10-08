//! Control grant mapping tied to the generated provider declaration.

use super::frame::Wire;
pub use super::interface::{Grant, grant_of};

impl Grant {
    pub const fn for_wire(wire: &Wire) -> Self {
        match wire {
            Wire::State(_) | Wire::StateInstance(_) => Self::State,
            Wire::Mint(_) => Self::Mint,
            Wire::Embark(_) | Wire::EmbarkInstance(_) => Self::Embark,
            Wire::Debark(_) | Wire::DebarkInstance(_) => Self::Debark,
            Wire::Ruin(_) | Wire::RuinInstance(_) => Self::Ruin,
        }
    }

    pub const fn of_wire(wire: &Wire) -> u8 {
        Self::for_wire(wire).at()
    }
}
