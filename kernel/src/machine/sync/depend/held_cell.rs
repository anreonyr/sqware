use ::core::cell::UnsafeCell;

use super::held_set::HeldSet;

pub(super) struct HeldCell(pub(super) UnsafeCell<HeldSet>);

unsafe impl Send for HeldCell {}
unsafe impl Sync for HeldCell {}
