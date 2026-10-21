//! RISC-V IOMMU（`riscv-iommu-sys`）的**用户态机制**与**独立驱动程序**：寄存器格、设备上下文、
//! Sv39 页表、命令/故障队列，以及一段经来源授权、由独立保留强持的硬件可达页窗口。
//!
//! 机制那一层只到“把表建起来、把队列立起来、编程、翻译、拆除并回读确认”，不含任何一景的判据：
//! `iommu-probe` / `iommu-dma` 两景是它的消费者。
//!
//! - [`regs`] / [`queue`] 碰 MMIO（读写寄存器、发 `IOFENCE`）；
//! - [`context`] / [`table`] 是纯填页（DDT 项、Sv39 项）；
//! - [`pages`] 经 `execution::memory` 取保留页并导出真实物理起址；
//! - [`order`] 是“本域写下的字节 ↔ 设备看到的字节”之间那一枚栅栏。
//!
//! # 这一台也是**驱动程序**（`main.rs` 那一张镜像的身子在这里）
//!
//! [`core`] / [`serve`] / [`api`] 是生产那条链上的 IOMMU 域：它自己持保留页与寄存器窗，
//! 自己编程目录表与队列，按协调入口（Hub）的请求逐笔完成
//! “取得帧引用 → 发布翻译 → 撤销 → 关队列 → 解除引用”。
//! PCI、设备恢复与设备协议都不在本域：本域不解释 virtio 寄存器，也不决定何时停端点。

pub mod api;
pub mod binding;
pub mod bindings;
pub mod context;
pub mod core;
pub mod fence;
pub mod handshake;
pub mod hold;
pub mod order;
mod outstanding;
pub mod pages;
pub mod phase;
pub mod queue;
pub mod references;
pub mod regs;
pub mod serve;
pub mod settled;
pub mod table;
