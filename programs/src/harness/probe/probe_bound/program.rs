//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// **上界的证客**：推一页 + 1、再推一枚不合族的帧到**持树者那几面**上
pub static PROBE_BOUND: UnitFile = UnitFile {
    name: "probe-bound",
    loader: crate::unit::LoaderConfig::user("probe-bound"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::UNIT,
            placement: crate::unit::operator::Placement::Instance,
            path: "x",
            scope: crate::unit::operator::Scope::Names(&["x"]),
            ops: system_api::operator::Ops::BIND.union(system_api::operator::Ops::FIND),
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
        supervision: None,
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Operator,
        )]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
