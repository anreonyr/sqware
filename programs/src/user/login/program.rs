use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROGRAM: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "login",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "terminal", "account", crate::unit::SCENE]),
        restart: Some(Ending::Told),
        ..Relation::DEFAULT
    },
    demand: Demand::DEFAULT,
};
