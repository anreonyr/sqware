use crate::core::memory::flat::Frame;
use alloc::{sync::Weak, vec::Vec};
use core::cell::Cell;

pub(super) struct Accessor {
    pub frames: Vec<Weak<Frame>>,
    pub active: Cell<bool>,
    pub timeout: Cell<bool>,
}

impl Accessor {
    pub fn write(&self, value: u8) -> bool {
        if !self.active.get() {
            return false;
        }
        for frame in &self.frames {
            let Some(frame) = frame.upgrade() else {
                return false;
            };
            // The temporary upgrade checks safety; the accessor retains no strong owner.
            unsafe {
                frame.as_mut_ptr().write_volatile(value);
            }
        }
        true
    }
}
