//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// Unified Operator sessions permit queries; Control-only mutations remain denied to this client.
pub static PROBE_OPERATOR_LAND: UnitFile = UnitFile {
    name: "probe-operator-land",
    loader: crate::unit::LoaderConfig::user("probe-operator-land"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "operator-fixture",
            scope: crate::unit::operator::Scope::Subtree,
            ops: system_api::operator::Ops::PART
                .union(system_api::operator::Ops::UNPART)
                .union(system_api::operator::Ops::BIND)
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
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
