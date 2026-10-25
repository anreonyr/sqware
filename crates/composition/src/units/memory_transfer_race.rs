//! 真实多 Hart 交错那一景两台（身子在本目录的 `main.rs` / `unit.rs`；"哪几台进哪张镜像"
//! 见 `:catalog` 的 `PROGRAMS`）。
//!
//! 管理端（`ENTRY`）与产品景里那几台自演台同款：**supervisor** 镜像、`OnSpawn`。
//! 对端（`UNIT`）是 **user** 镜像：它由管理端装载进一个**新 Team**（故它有 sire），
//! 那一 Team 的成员带着收据——它真退出时因此不会被当成无人认领的塌方。

use crate::unit::UnitFile;

pub static ENTRY: UnitFile = UnitFile {
    name: "memory-transfer-race",
    loader: crate::unit::LoaderConfig::supervisor("memory-transfer-race"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

pub static UNIT: UnitFile = UnitFile {
    name: "memory-transfer-race-unit",
    loader: crate::unit::LoaderConfig::user("memory-transfer-race-unit"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
