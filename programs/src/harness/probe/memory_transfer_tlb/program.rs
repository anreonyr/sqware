//! 那一台引导镜像的声明（身子在本目录的 `main.rs`）。
//!
//! 它是 **supervisor** 镜像：读者与它同 Team 同空间，故读者也在 S 态跑——
//! 那一段热窗口要读 `stvec`、要清 `sstatus.SIE`、要装自己的 Direct 向量，
//! 这三件在 U 态做不了。现有 D1 那一景的 `memory-transfer-unit` 仍是不升权的 user。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "memory-transfer-tlb",
    loader: crate::unit::LoaderConfig::supervisor("memory-transfer-tlb"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
