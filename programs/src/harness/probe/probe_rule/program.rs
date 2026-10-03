//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROBE_RULE: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: Some(crate::harness::probe::hierarchy::publication),
    #[cfg(target_arch = "riscv64")]
    prepare: None,
    identity: Identity {
        name: "probe-rule",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "identity"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // **答得动**：`probe-rule-other` 读的那几格由本台落——落完才交这一枚（与三台驱动同一手）。
        ..Demand::DEFAULT
    },
};
