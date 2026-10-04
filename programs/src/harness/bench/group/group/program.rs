//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Identity, Relation, UnitFile};
use env::ProgramKind;

pub static GROUP: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "group",
        space: ProgramKind::Supervisor,
        wanted_by: &["group"],
        entry: &["group"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
