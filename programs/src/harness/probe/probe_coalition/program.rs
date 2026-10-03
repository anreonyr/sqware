//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// Unified Identity publication acceptance: seventeen actions, action marks and one authority.
/// The historical executable name remains stable for existing acceptance runners.
pub static PROBE_COALITION: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: None,
    #[cfg(target_arch = "riscv64")]
    prepare: Some(crate::harness::probe::identity::supply_to),
    identity: Identity {
        name: "probe-coalition",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "identity"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
