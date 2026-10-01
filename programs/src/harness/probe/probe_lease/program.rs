//! probe_lease — **这一台**的装配声明（身子在本目录的 `main.rs`；"哪几台进哪张镜像"见
//! [`crate::unit::catalog`] 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 会死的持有者：落一块**声明归自己**的门牌然后直接死，好让下一台接手。
pub static PROBE_LEASE: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-lease",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
