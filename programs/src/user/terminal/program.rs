//! terminal::program — **控制台那一台**（`prog-terminal`）的装配声明。
//! **U 态**（最小特权）：只走树上那一族客手与 `env` 的调试面，够不着建域那道 S 态门。

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手哪一步。
pub const E_TERMINAL: Died = 24;

pub static PROGRAM: UnitFile = UnitFile {
    publication: &[crate::unit::Publish::Entries {
        scope: crate::unit::PublishScope::Terminal,
        group: "",
        road: "svc/terminal",
        entries: &crate::unit::PublishEntry::from_names(terminal_api::PUBLICATIONS),
        public: true,
    }],
    identity: Identity {
        name: "terminal",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "uart"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
