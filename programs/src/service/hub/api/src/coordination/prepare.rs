use env::{PieToken, TaskId};

use crate::frame::BindingId;

/// Hub asks a driver to prepare the hardware path for one qualified binding.
/// The receiver must authenticate `hub` against the kernel sender before effects.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 41)]
pub struct Prepare {
    pub op: u8,
    pub hub: TaskId,
    pub serial: u64,
    pub key: u64,
    pub execution: TaskId,
    pub back: PieToken,
}

impl Prepare {
    pub const fn binding(self) -> BindingId {
        BindingId {
            hub: self.hub,
            serial: self.serial,
        }
    }
}
