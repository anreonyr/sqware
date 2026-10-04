//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// Ordinary bound task tests installer sender authorization and action-face isolation.
pub static PROBE_DENIED: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "probe-denied",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "identity"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
