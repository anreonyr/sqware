//! canonical::program — **控制台那一台**（`prog-canonical`）的装配声明。
//!
//! **U 态**（最小特权）：只走树上那一族客手与 `env` 的调试面，够不着建域那道 S 态门。
//! 它进 `root` / `product` 两景（原先那台回显的位置），且是那两景**最后一条**（`order: 18`）
//! ——编排域等它退场才收场（它一退 ⇒ 引导域退 ⇒ 级联扑杀 ⇒ 停机）。

use crate::program::{Died, Program, Spot};
use env::ProgramKind;

/// 它死在起手哪一步。
pub const E_CANONICAL: Died = 24;

pub static PROGRAM: Program = Program {
    name: "canonical",
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
    died: E_CANONICAL,
    setup: &[],
};
