//! 起手受控失败那一景（发布与武装都过了、交付之前）System 的装配声明。
//!
//! 它与 `system` 是两台**不同镜像**：景的入口按镜像名认，故每景一份声明，
//! 景的装配差异（在哪一个受控点停）不会外溢到普通产品。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "system-start-after-dma",
    loader: crate::unit::LoaderConfig::supervisor("system-start-after-dma"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig::DEFAULT,
    supply: crate::unit::SupplyConfig::DEFAULT,
};
