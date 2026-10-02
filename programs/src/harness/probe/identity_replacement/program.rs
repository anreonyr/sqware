use crate::unit::{Demand, Ending, Identity, Relation, Setup, UnitFile};

pub static ENTRY: UnitFile = UnitFile {
    identity: Identity {
        name: "identity-replacement",
        wanted_by: &["identity-replacement"],
        entry: &["identity-replacement"],
        space: env::ProgramKind::Supervisor,
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static DEPENDENT: UnitFile = UnitFile {
    identity: Identity {
        name: "replacement-dependent",
        wanted_by: &["identity-replacement"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["identity"]),
        restart: Some(Ending::Resident),
    },
    demand: Demand { supply: &[Setup::Ready] },
};

pub static CHILD: UnitFile = UnitFile {
    identity: Identity {
        name: "replacement-child",
        wanted_by: &["identity-replacement"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["identity"]),
        restart: Some(Ending::Resident),
    },
    demand: Demand { supply: &[Setup::Ready] },
};
