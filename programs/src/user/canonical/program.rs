//! canonical::program — **控制台那一台**（`prog-canonical`）的装配声明。
//! **U 态**（最小特权）：只走树上那一族客手与 `env` 的调试面，够不着建域那道 S 态门。

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手哪一步。
pub const E_CANONICAL: Died = 24;

pub static PROGRAM: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "canonical",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "uart"]),
        restart: Some(Ending::Told),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
