use crate::unit::{Demand, Identity, Relation, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "fail",
        wanted_by: &["accept", "product", "system-fault"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
