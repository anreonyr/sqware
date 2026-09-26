//! uart::program — **串口驱动**（`prog-uart`）的装配声明。
//!
//! **U 态**：持有 `serial@10000000`（banner 里那张 PMP 是 S/U (R,W)），把"收到字节就拉线"打开。

use crate::program::{Died, Program, Setup, Spot};
use env::supply::{Kind, Need, class_block};
use env::{Access, Policy, ProgramKind};

/// 它死在起手 / 常驻哪一步。
pub const E_UART: Died = 9;

/// 串口驱动要的那一枚：**那一台 `ns16550a`**。
pub const UART_WANTS: &[Need] = &[Need::class(
    class_block("ns16550a"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];

pub static PROGRAM: Program = Program {
    name: "uart",
    kind: ProgramKind::User,
    spot: Spot::Service,
    scenes: &["root", "product"],
    entry: &[],
    order: Some(4),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_UART,
    setup: &[Setup::Need(UART_WANTS[0]), Setup::Channel("records")],
};
