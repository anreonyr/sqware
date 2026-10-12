//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

pub static PROBE_RULE: UnitFile = UnitFile {
    name: "probe-rule",
    loader: crate::unit::LoaderConfig::user("probe-rule"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "rule",
            scope: crate::unit::operator::Scope::Names(&[
                "is",
                "under",
                "in",
                "door",
                "open",
                "foreign",
                "temp",
                "at-pane",
                "gone-door",
                "mine",
            ]),
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
        grants: &[
            system_api::identity::Grant::Adopt, system_api::identity::Grant::Waive,
            system_api::identity::Grant::Restrict, system_api::identity::Grant::Derive,
            system_api::identity::Grant::Found, system_api::identity::Grant::Admit,
            system_api::identity::Grant::Expel,
        ],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Transient),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
