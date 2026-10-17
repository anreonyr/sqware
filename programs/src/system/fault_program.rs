//! 故障注入那一景 System 的装配声明（身子同 `system/main.rs` 那一台）。
//!
//! 它与 `system` 是两台**不同镜像**：景的入口按镜像名认，故两份声明各自成一份，
//! 景的装配差异（第二轮注入）不会外溢到普通产品。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "system-fault-dma",
    loader: crate::unit::LoaderConfig::supervisor("system-fault-dma"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
