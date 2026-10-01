//! router::program — **线路由者**（`prog-router`）的装配声明。
//!
//! **U 态**：它只读 PLIC 的寄存器、claim/complete、铸孔、挂组，全都不需要 S 态；它那枚铃是
//! **内核给的**（铸铃那一格才是 S 态，本域不铸）。

use crate::unit::{Demand, Died, Ending, Identity, UnitFile, Relation, Setup};

/// 它死在起手 / 常驻哪一步。
pub const E_ROUTER: Died = 5;

/// 中断控制器那一类（`compatible`）——**"我是哪台控制器"这个断言只有一处**：线路由域认设备树
/// 时读它（`driver/router/core/sources.rs`），下面这张单子要的也是它。
pub const PLIC_CLASS: &str = "sifive,plic-1.0.0";

// **照实记（"要的那三样"搬回本域）**：这一份原先还开着本域那张需求单（`ROUTER_WANTS`），而装配者
// 按同一张单替本域领三样（控制器 / 设备树 / 门铃）。那一整条路退了 ⇒ 三张单回了本域自己
// （`driver/router/adapt/boot.rs` 的三条 `Ask`）；**留下的 `PLIC_CLASS` 仍是本域的事实**——
// 它同时是"我是哪台控制器"那句断言（读树那一侧也用同一枚常量，见 `system/machine.rs`）。

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "router",
        wanted_by: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "hub"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // **起手最后一步（落面）之后才交**：这一格就是「答得动」的凭据。
        ..Demand::DEFAULT
    },
};
