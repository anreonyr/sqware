//! 实时钟驱动（prog-rtc）的装配声明。
//! **U 态**：持有 `rtc@101000`（11 号线），武装闹钟、到点自己拉线；客人定的闹钟到点就清掉
//! 那一格、把"那一声"推回去。它是"抽象等第二个实例"的那个第二例。

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手 / 常驻哪一步。
pub const E_RTC: Died = 12;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "rtc",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "hub", "router"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
