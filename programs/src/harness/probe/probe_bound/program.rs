//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! crate::unit::catalog 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **上界的证客**：推一页 + 1、再推一枚不合族的帧到**持树者那几面**上。
pub static PROBE_BOUND: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-bound",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
