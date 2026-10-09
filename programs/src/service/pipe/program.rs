use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    publication: &[crate::unit::Publish::Entries {
        scope: crate::unit::PublishScope(6),
        group: "",
        road: "svc/pipe",
        entries: &[crate::unit::PublishEntry { name: "create" }],
        public: true,
    }],
    identity: Identity {
        name: "pipe",
        wanted_by: &["accept", "product", "system-fault"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "identity"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand::DEFAULT,
};
