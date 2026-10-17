//! 端点侧收口要的三样。

use crate::driver::pci::endpoint::Endpoint;
use crate::driver::pci::virtio::Caps;

/// 端点侧收口要的三样：已配置的端点（清 bus-master），BAR 的本域基址与能力表（复位并回读）。
#[derive(Clone, Copy)]
pub struct EndpointDrain {
    pub endpoint: Endpoint,
    pub bar: usize,
    pub caps: Caps,
}
