//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **"退场即撤订"那一对的前半**：订一条路、停一下、退场（见 `main.rs` 头注）。
///
/// 次序：树那条路（`operator`）。与 `probe-watch-after` **并发**（两条边不互相等）。
pub static PROBE_WATCH_GONE: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "probe-watch-gone",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
