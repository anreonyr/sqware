//! echo::program — **调试回显**（`prog-echo`）的装配声明。
//!
//! **U 态**（最小特权）：只走 `env` 的调试面（`DebugCall`），够不着建域那道 S 态门。
//! 产品镜像里它排**最后一条**（`order: 18`，原 17）——编排域等它退场才收场。

use crate::program::{Died, Program, Spot};
use env::ProgramKind;

/// 它死在起手哪一步。
pub const E_ECHO: Died = 6;

pub static PROGRAM: Program = Program {
    name: "echo",
    kind: ProgramKind::User,
    spot: Spot::Console,
    scenes: &["root", "product"],
    entry: &[],
    order: Some(18),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: None,
    died: E_ECHO,
    setup: &[],
};
