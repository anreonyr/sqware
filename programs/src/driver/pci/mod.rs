//! PCI 驱动那一台：**独立 Task/Space** 里的总线枚举、BAR 落位与 MASTER。
//!
//! 与 [`crate::driver::iommu`] 分开：这里是**端点设备**（`virtio-blk-pci`）在总线
//! 那一侧的面，那里是**翻译单元**（IOMMU）的面；设备协议（virtio 寄存器）归设备
//! 恢复方与执行实例，不在这里。
//!
//! - [`snapshot`] / [`scan`]：只读 256 字节快照与有界通用枚举；
//! - [`ecam`] / [`endpoint`]：配置空间、探尺与落位（只开内存空间）；
//! - [`virtio`]：能力表四格与队列寄存器（设备那一侧用）；
//! - [`api`] / [`bindings`] / [`core`] / [`serve`]：本台那一域的门、声明窗与核心。

pub mod api;
pub mod bindings;
pub mod core;
pub mod ecam;
pub mod end;
pub mod endpoint;
pub mod function;
pub mod page;
pub mod placement;
pub mod reader;
pub mod scan;
pub mod serve;
pub mod snapshot;
pub mod virtio;
pub mod window;
