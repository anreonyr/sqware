use crate::unit::{Demand, Identity, Relation, UnitFile};

pub static PROGRAM: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "accept-system",
        space: env::ProgramKind::Supervisor,
        wanted_by: &["accept"],
        entry: &["accept"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
