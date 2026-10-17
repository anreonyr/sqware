//! RISC-V IOMMU（`riscv-iommu-sys`）的**用户态机制**：寄存器格、设备上下文、Sv39 页表、
//! 命令/故障队列，以及一段经来源授权、由独立保留强持的硬件可达页窗口。
//!
//! 这里只到“把表建起来、把队列立起来、编程、翻译、拆除并回读确认”，不含任何一景的判据：
//! `iommu-probe` / `iommu-dma` 两景，以及将来 System 侧的 `Platform`，都是它的消费者。
//!
//! - [`regs`] / [`queue`] 碰 MMIO（读写寄存器、发 `IOFENCE`）；
//! - [`context`] / [`table`] 是纯填页（DDT 项、Sv39 项）；
//! - [`pages`] 经 `execution::memory` 取保留页并导出真实物理起址；
//! - [`order`] 是“本域写下的字节 ↔ 设备看到的字节”之间那一枚栅栏。

pub mod context;
pub mod order;
pub mod pages;
pub mod queue;
pub mod regs;
pub mod table;
