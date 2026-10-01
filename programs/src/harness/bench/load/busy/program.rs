//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! crate::unit::catalog 的 `PROGRAMS`）。

use crate::unit::{Demand, Identity, Relation, UnitFile};

pub static BUSY: UnitFile = UnitFile {
    identity: Identity {
        name: "busy",
        wanted_by: &["load"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
