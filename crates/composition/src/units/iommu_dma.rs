//! 端点在 IOMMU 翻译下真做 DMA 那一景的声明（身子在 `dma/main.rs`）。
//!
//! 它是 **supervisor** 镜像：要从引导账取 IOMMU 段、ECAM 段与 PCI 窗口三枚门闩，
//! 还要 retain 页并从 `traverse` 读回真实物理页起址。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "iommu-dma",
    loader: crate::unit::LoaderConfig::supervisor("iommu-dma"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
