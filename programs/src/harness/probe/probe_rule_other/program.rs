//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// 有身份地去用别人立了规矩的那两格 ⇒ 都该拒（第二道门的反例）
pub static PROBE_RULE_OTHER: UnitFile = UnitFile {
    name: "probe-rule-other",
    loader: crate::unit::LoaderConfig::user("probe-rule-other"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Unit("probe-rule"),
        ]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};
