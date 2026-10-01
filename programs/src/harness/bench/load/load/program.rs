//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Identity, Relation, UnitFile};
use env::ProgramKind;

pub static LOAD: UnitFile = UnitFile {
    identity: Identity {
        name: "load",
        space: ProgramKind::Supervisor,
        wanted_by: &["load"],
        entry: &["load"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
