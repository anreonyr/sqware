//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// Unified Operator sessions permit queries; Control-only mutations remain denied to this client.
pub static PROBE_OPERATOR_LAND: UnitFile = UnitFile {
    publication: &[crate::unit::Publish::Entries {
        scope: crate::unit::PublishScope::Fixture,
        group: "operator-fixture",
        road: "svc/operator-fixture",
        entries: &[crate::unit::PublishEntry { name: "entry" }],
        public: false,
    }],
    identity: Identity {
        name: "probe-operator-land",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
