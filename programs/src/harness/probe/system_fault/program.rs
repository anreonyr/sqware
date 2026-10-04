use crate::unit::{Demand, Ending, Identity, Relation, Setup, UnitFile};

pub static ENTRY: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "system-fault",
        wanted_by: &["system-fault"],
        entry: &["system-fault"],
        space: env::ProgramKind::Supervisor,
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};

pub static DEPENDENT: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "system-dependent",
        wanted_by: &["system-fault"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["identity"]),
        restart: Some(Ending::Resident),
    },
    demand: Demand {
        supply: &[Setup::Ready],
    },
};

pub static CHILD: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "system-child",
        wanted_by: &["system-fault"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["identity"]),
        restart: Some(Ending::Resident),
    },
    demand: Demand {
        supply: &[Setup::Ready],
    },
};

pub static FAULT_UNIT: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "system-fault-unit",
        wanted_by: &["system-fault"],
        space: env::ProgramKind::Supervisor,
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
