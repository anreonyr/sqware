use env::{Mark, PieToken};

use crate::frame::DmaSaid;
use super::Wire;

/// Driver coordination entry in the SERVICE namespace.
pub const ENTRY_PATH: &str = "device/coord";
pub const ENTRY: Mark = Mark::of(ENTRY_PATH);
pub const BACK: Mark = Mark::of("device/coord-back");

fn reply_to(wire: &Wire) -> PieToken {
    wire.back()
}

#[mold::contract(request = Wire, response = DmaSaid, mark = BACK, back = reply_to)]
pub struct Call;
