//! 串口驱动（prog-uart）的装配声明。
//! **U 态**：持有 `serial@10000000`（banner 里那张 PMP 是 S/U (R,W)），把"收到字节就拉线"打开。

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手 / 常驻哪一步
pub const E_UART: Died = 9;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "uart",
        wanted_by: &["accept", "product", "identity-replacement"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "hub", "router"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
