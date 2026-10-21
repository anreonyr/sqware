//! context（那一域的门面与逐客）· device（设备面那几手：
//! 问账、借映、读写）· fail（一族口径：这一域死在起手/常驻的哪一步）。
//! dma（DMA 绑定的坐标与 Hub 那一面的客户）· peer（类型化对端客户面）·
//! platform（受限平台窗的接收侧）· retention（声明保留租约的接收侧）。

pub mod context;
pub mod device;
pub mod dma;
pub mod fail;
pub mod peer;
pub mod platform;
pub mod retention;
