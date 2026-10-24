//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// 主体：身份服务的第一位真客人
pub static SUBJECT: UnitFile = UnitFile {
    name: "subject",
    loader: crate::unit::LoaderConfig::user("subject"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[
            system_api::identity::Grant::Adopt,
            system_api::identity::Grant::Waive,
            system_api::identity::Grant::Restrict,
            system_api::identity::Grant::Derive,
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
