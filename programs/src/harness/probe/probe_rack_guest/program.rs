//! probe-rack-guest 的装配声明（身子在本目录 `main.rs`）。
//!
//! **对端那一侧**：用产品同一条客人面取回两端，先读完 A、再写满 B。
//! 次序：**`operator` 必须写在 `after` 里**——装配者那一手 `attach` 只对"点了树那位"的台做
//! （`operator::bridge::needs_tree`），而本台要开会话（交流那一枚 `LINK`）就得有人跟它对上。
//! 再等铺场那一台（`probe-rack-mount`）：它落完两枚砖、把 A 写满之后才响 `Ready`。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROBE_RACK_GUEST: UnitFile = UnitFile {
    identity: Identity {
        name: "probe-rack-guest",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "probe-rack-mount"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
