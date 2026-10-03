//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 有身份地去用别人立了规矩的那两格 ⇒ 都该拒（第二道门的反例）
pub static PROBE_RULE_OTHER: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: None,
    #[cfg(target_arch = "riscv64")]
    prepare: None,
    identity: Identity {
        name: "probe-rule-other",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "probe-rule"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
