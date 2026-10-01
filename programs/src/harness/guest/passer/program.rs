//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 过客：起来、挂一个名字、**直接死**（不说再见）
pub static PASSER: UnitFile = UnitFile {
    identity: Identity {
        name: "passer",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&[]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
