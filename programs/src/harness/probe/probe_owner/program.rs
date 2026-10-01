//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! crate::unit::catalog 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **有身份**、但那一格归别人（声明过归属）⇒ 也拒。
pub static PROBE_OWNER: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-owner",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "uart"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
