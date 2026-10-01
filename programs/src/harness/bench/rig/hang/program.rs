//! hang — **这一台**的装配声明（身子在本目录的 `main.rs`；"哪几台进哪张镜像"见
//! [`crate::unit::catalog`] 的 `PROGRAMS`）。

use crate::unit::{Demand, Identity, Relation, UnitFile};

pub static HANG: UnitFile = UnitFile {
    identity: Identity {
        name: "hang",
        wanted_by: &["rig"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
