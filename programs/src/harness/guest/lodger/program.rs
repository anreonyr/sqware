//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

pub const E_LODGER: Died = 11;

/// 房客：占一条线、**直接死**——线路由者那本账的探活读数
pub static LODGER: UnitFile = UnitFile {
    identity: Identity {
        name: "lodger",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "hub", "router"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
