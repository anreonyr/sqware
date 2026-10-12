//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::UnitFile;

pub static GROUP: UnitFile = UnitFile {
    name: "group",
    loader: crate::unit::LoaderConfig::supervisor("group"),
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
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
