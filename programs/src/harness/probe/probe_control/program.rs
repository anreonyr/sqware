//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, SCENE, UnitFile};

/// 验证 Control 的公开查询面和受限写入面。
pub static PROBE_CONTROL: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "probe-control",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", SCENE]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
