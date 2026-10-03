//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Identity, Relation, UnitFile};
use env::ProgramKind;

pub static RIG: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "rig",
        space: ProgramKind::Supervisor,
        wanted_by: &["rig"],
        entry: &["rig"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
