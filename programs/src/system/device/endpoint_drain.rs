//! 端点侧那一条收口要的四样。

use crate::driver::pci::virtio::Caps;

/// 端点侧那一条收口要的四样：ECAM 段起址与功能号（清 bus-master），
/// 以及那一根 BAR 的本域基址与能力表（按 virtio 那一手复位并回读确认）。
#[derive(Clone, Copy)]
pub struct EndpointDrain {
    pub ecam: usize,
    pub requester: u16,
    pub bar: usize,
    pub caps: Caps,
}
