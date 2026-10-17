//! 端点服务那一台的装配声明（身子在 `endpoint.rs`）。
//!
//! 它**不在任何景的静态启动单上**：由 System 的设备管理者按需建 Team、按需起它，
//! 并在放开之前把坐标经那一枚孔推过去。故它没有 `after`、也不按名册等任何东西。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "system-dma",
    loader: crate::unit::LoaderConfig::supervisor("system-dma"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
