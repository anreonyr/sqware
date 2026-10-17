//! 那一台引导镜像的声明（身子在本目录的 `main.rs`）。
//!
//! 它是 **supervisor** 镜像：控制者与它起的读者同 Team 同空间，读者对外只做普通
//! 访存与握手页读写，升降权不是本景需要的——与 `memory-transfer-tlb` 同款选 supervisor，
//! 不为本景另铸一台被装载的控制单元。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "memory-transfer-fresh",
    loader: crate::unit::LoaderConfig::supervisor("memory-transfer-fresh"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
