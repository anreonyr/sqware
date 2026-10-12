//! probe-rack-mount 的装配声明（身子在本目录 `main.rs`）。
//!
//! **铺场那一侧**：开两具架、落**两枚**砖、把 A 写满、响 `Ready`，并在收尾封印自己那一枚页（验树剔死那一格）。
//! 它要给 `Ready` 凭据——客人那一台的 `after` 指着它（`UnitFile::supply` 按这条边推出来）。

use crate::unit::{Ending, UnitFile};

pub static PROBE_RACK_MOUNT: UnitFile = UnitFile {
    name: "probe-rack-mount",
    loader: crate::unit::LoaderConfig::user("probe-rack-mount"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            path: "probe-rack",
            scope: crate::unit::operator::Scope::Names(&["rx", "tx"]),
            ops: system_api::operator::Ops::BIND
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: None,
            source_mark: None,
            bindings: &[],
        }],
    },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Operator,
        )]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Transient),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
