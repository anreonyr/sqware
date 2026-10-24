//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Ending, UnitFile};

/// Unified Identity publication acceptance: twenty actions, action marks and one authority.
/// The historical executable name remains stable for existing acceptance runners.
pub static PROBE_COALITION: UnitFile = UnitFile {
    name: "probe-coalition",
    loader: crate::unit::LoaderConfig::user("probe-coalition"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[
            system_api::identity::Grant::Adopt,
            system_api::identity::Grant::Waive,
            system_api::identity::Grant::Restrict,
            system_api::identity::Grant::Derive,
            system_api::identity::Grant::Found,
            system_api::identity::Grant::Admit,
            system_api::identity::Grant::Expel,
            system_api::identity::Grant::Activate,
        ],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
