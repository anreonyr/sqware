use alloc::vec::Vec;

use super::region_id::RegionId;
use super::slot::Slot;

pub(super) struct Region {
    pub(super) id: RegionId,
    pub(super) base: usize,
    pub(super) edge: usize,
    pub(super) reserved: Vec<Slot>,
}
