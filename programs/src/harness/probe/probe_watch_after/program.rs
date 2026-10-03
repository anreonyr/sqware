//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **"退场即撤订"那一对的后半**：在同一条路上连改几趟（见 `main.rs` 头注）。
///
/// 次序：树那条路（`operator`）。**不声明等 `probe-watch-gone`**：`after` 只等到"答得动"，
/// 等不到"退场"——两台的判据靠"连改跨过那一刻"成立。
pub static PROBE_WATCH_AFTER: UnitFile = UnitFile {
    publication: &[crate::unit::Publish::Entries {
        scope: crate::unit::PublishScope::Fixture, group: "probe-swatch", road: "svc/probe-swatch",
        entries: &[
            crate::unit::PublishEntry { name: "in", mark: None },
        ], public: false,
    }],
    identity: Identity {
        name: "probe-watch-after",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
