//! Identity local readiness is distinct from Control's complete path publication.
use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

pub const E_IDENTITY: Died = 14;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "identity",
        wanted_by: &["accept", "product", "identity-replacement"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
