use alloc::sync::Arc;

use super::reservation::Reservation;
use crate::core::memory::manager::addr::VirtAddr;
use crate::core::memory::manager::tree::Map;

/// 一笔已落地的映射：`Map` 是稳定身份，`Reservation` 是它在地址桶里的保留。
pub struct Mapping {
    pub(crate) map: Arc<Map>,
    pub(crate) reservation: Reservation,
}

impl Mapping {
    pub fn address(&self) -> VirtAddr {
        self.reservation.start()
    }

    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.reservation.size()
    }
}

unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}
