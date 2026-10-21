//! probe-rack-guest 的装配声明（身子在本目录 `main.rs`）。
//!
//! 依赖 Operator 会话和已发布两端的 probe-rack-mount。
//! 取回两端后先读完 A，再写满 B。

use crate::unit::{Ending, UnitFile};

pub static PROBE_RACK_GUEST: UnitFile = UnitFile {
    name: "probe-rack-guest",
    loader: crate::unit::LoaderConfig::user("probe-rack-guest"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Unit("probe-rack-mount"),
        ]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
