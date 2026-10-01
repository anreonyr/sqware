//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! crate::unit::catalog 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **操作面的正证客人（只有 `land` 一位那一半）**：会话开在 `granted_berth(Land)` 上，
/// 于是 `seek` / `part` / `find` / `trim` 全答 `Denied`，而 `land` 在**无主**那一格上通、
/// 在**别人有主**那一格上拒——后者证的是"面判与归属那一条轴**正交**"。
pub static PROBE_OPERATOR_LAND: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-operator-land",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
