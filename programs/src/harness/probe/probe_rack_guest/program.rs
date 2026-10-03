//! probe-rack-guest 的装配声明（身子在本目录 `main.rs`）。
//!
//! 依赖 Operator 会话和已发布两端的 probe-rack-mount。
//! 取回两端后先读完 A，再写满 B。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROBE_RACK_GUEST: UnitFile = UnitFile {
    publication: &[],
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
