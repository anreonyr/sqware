//! 那一台引导镜像的声明（身子在同目录的 `main.rs`）。
//!
//! 它是 **supervisor** 镜像：要从引导账取设备区门闩，还要 retain 页并从 `traverse`
//! 读回真实物理页起址。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "iommu-probe",
    loader: crate::unit::LoaderConfig::supervisor("iommu-probe"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
