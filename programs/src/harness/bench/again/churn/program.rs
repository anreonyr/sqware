//! churn — **这一台**的装配声明（身子在本目录的 `main.rs`；"哪几台进哪张镜像"见
//! [`crate::unit::catalog`] 的 `PROGRAMS`）。

use crate::unit::{Demand, Identity, Relation, UnitFile};

pub static CHURN: UnitFile = UnitFile {
    identity: Identity {
        name: "churn",
        wanted_by: &["again"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
