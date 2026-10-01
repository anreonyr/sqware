//! 线路由者（prog-router）的装配声明。
//! **U 态**：它只读 PLIC 的寄存器、claim/complete、铸孔、挂组，全都不需要 S 态；它那枚铃是

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手 / 常驻哪一步。
pub const E_ROUTER: Died = 5;

/// 中断控制器那一类（`compatible`）——**"我是哪台控制器"这个断言只有一处**：线路由域认设备树
/// 时读它（`driver/router/core/sources.rs`），下面这张单子要的也是它。
pub const PLIC_CLASS: &str = "sifive,plic-1.0.0";

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "router",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "hub"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
