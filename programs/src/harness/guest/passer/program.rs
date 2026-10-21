//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// 过客：起来、挂一个名字、**直接死**（不说再见）
pub static PASSER: UnitFile = UnitFile {
    name: "passer",
    loader: crate::unit::LoaderConfig::user("passer"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
