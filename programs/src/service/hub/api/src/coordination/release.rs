use env::{PieToken, TaskId};

use crate::frame::BindingId;

/// Hub asks a driver to release the hardware path for one qualified binding.
/// The receiver must authenticate the sender and match the stored binding.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 42)]
pub struct Release {
    pub op: u8,
    pub hub: TaskId,
    pub serial: u64,
    pub key: u64,
    pub execution: TaskId,
    pub reason: u8,
    pub back: PieToken,
}

impl Release {
    pub const fn binding(self) -> BindingId {
        BindingId {
            hub: self.hub,
            serial: self.serial,
        }
    }
}
