use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "probe-terminal",
        wanted_by: &["accept"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "terminal"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand::DEFAULT,
};
