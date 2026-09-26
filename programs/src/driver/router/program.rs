//! router::program — **线路由者**（`prog-router`）的装配声明。
//!
//! **U 态**：它只读 PLIC 的寄存器、claim/complete、铸孔、挂组，全都不需要 S 态；它那枚铃是
//! **内核给的**（铸铃那一格才是 S 态，本域不铸）。

use crate::program::{Died, Program, Setup, Spot};
use env::supply::{Kind, Need, class_block};
use env::{Access, Key, Policy, ProgramKind};

/// 它死在起手 / 常驻哪一步。
pub const E_ROUTER: Died = 5;

/// 中断控制器那一类（`compatible`）——**"我是哪台控制器"这个断言只有一处**：线路由域认设备树
/// 时读它（`driver/router/core/sources.rs`），下面这张单子要的也是它。
pub const PLIC_CLASS: &str = "sifive,plic-1.0.0";

/// 线路由者要的那三样：**中断控制器**（按类要）＋ **设备树本体 / 门铃**（boot 造的，按已知坐标）。
pub const ROUTER_WANTS: &[Need] = &[
    Need::class(
        class_block(PLIC_CLASS),
        Kind::Pole,
        Access::FETCH_STORE,
        Policy::ONLY,
    ),
    Need::known(Key::dtb(), Kind::Pole, Access::FETCH, Policy::NONE),
    Need::known(Key::irq(), Kind::Nole, Access::FETCH, Policy::NONE),
];

pub static PROGRAM: Program = Program {
    name: "router",
    kind: ProgramKind::User,
    spot: Spot::Service,
    scenes: &["root", "product"],
    entry: &[],
    order: Some(3),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_ROUTER,
    // 单子的次序 = 回单的次序（收方按位次归位）：三枚门闩，再一条通道。
    setup: &[
        Setup::Need(ROUTER_WANTS[0]),
        Setup::Need(ROUTER_WANTS[1]),
        Setup::Need(ROUTER_WANTS[2]),
        Setup::Channel("records"),
    ],
};
