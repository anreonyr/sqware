//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// **有身份**、但那一格归别人（声明过归属）⇒ 也拒
pub static PROBE_OWNER: UnitFile = UnitFile {
    name: "probe-owner",
    loader: crate::unit::LoaderConfig::user("probe-owner"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Unit("uart"),
        ]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
