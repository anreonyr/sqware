use crate::unit::{Demand, Identity, Relation, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "shell",
        wanted_by: &["accept", "product", "system-fault"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
