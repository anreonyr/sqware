//! PCI 端点那一侧的**用户态机制**：ECAM 配置空间、一棵内存 BAR 的落位，
//! 以及 virtio-pci 1.x 的能力表与队列。
//!
//! 与 [`crate::driver::iommu`] 分开：这里是**端点设备**（`virtio-blk-pci`）的面，
//! 那里是**翻译单元**（IOMMU）的面；把两边接起来的是具体的景/产品。

pub mod ecam;
pub mod endpoint;
pub mod virtio;
