//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROBE_RULE: UnitFile = UnitFile {
    publication: &[crate::unit::Publish::Entries {
        scope: crate::unit::PublishScope(4),
        group: "rule",
        road: "svc/rule",
        entries: &[
            crate::unit::PublishEntry {
                name: "is",
            },
            crate::unit::PublishEntry {
                name: "under",
            },
            crate::unit::PublishEntry {
                name: "in",
            },
            crate::unit::PublishEntry {
                name: "door",
            },
            crate::unit::PublishEntry {
                name: "open",
            },
            crate::unit::PublishEntry {
                name: "foreign",
            },
            crate::unit::PublishEntry {
                name: "temp",
            },
            crate::unit::PublishEntry {
                name: "at-pane",
            },
            crate::unit::PublishEntry {
                name: "gone-door",
            },
            crate::unit::PublishEntry {
                name: "mine",
            },
        ],
        public: false,
    }],
    identity: Identity {
        name: "probe-rule",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "identity"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // **答得动**：`probe-rule-other` 读的那几格由本台落——落完才交这一枚（与三台驱动同一手）。
        ..Demand::DEFAULT
    },
};
