//! 真实 Task 等待那一景两台（身子在本目录的 `main.rs` / `unit.rs`；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`）。
//!
//! 引导那一台（`ENTRY`）与产品景里那几台自演台同款：**supervisor** 镜像、`OnSpawn`。
//! 控制单元（`UNIT`）是**user** 镜像：它由引导那一台装载进一台新 Team（故它有 sire），
//! 那一 Team 的成员带着收据——真实页错误终止的任务因此不会被当成无人认领的塌方。

use crate::unit::UnitFile;

pub static ENTRY: UnitFile = UnitFile {
    name: "memory-transfer",
    loader: crate::unit::LoaderConfig::supervisor("memory-transfer"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

pub static UNIT: UnitFile = UnitFile {
    name: "memory-transfer-unit",
    loader: crate::unit::LoaderConfig::user("memory-transfer-unit"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
