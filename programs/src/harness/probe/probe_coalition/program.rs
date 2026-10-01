//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! `../unit/catalog.rs` 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **那两族"没有会话"的服务**（名册 / 盟册）的格数判据：`/svc/sys/{principal,coalition}` 底下
/// 各该有 `Grant::ALL.len()` 枚（一枚 Grant = 一枚门牌 = 一格），且那四枚门牌都取得回来。
///
/// 为什么两族同一台：它们**逐字同形**（无会话、门牌即路、两面各一枚孔），共用的判据也同形；
/// 分开两台就是同一份正文写两遍。`operator` 那一族已由 `probe-operator-gate` 数、`control`
/// 那一族已由 `probe-control` 数——**四族格数因此各归各家**。
///
/// 次序：那两族起的头（`after: Some(&["operator", "principal", "coalition"])`）；`operator` 必须在，
/// 因为上树要它那条路（与 `probe-control` 同一条依赖）。
pub static PROBE_COALITION: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-coalition",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "principal", "coalition"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
