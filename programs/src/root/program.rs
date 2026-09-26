//! root::program — **引导域**（`prog-root`）的装配声明。
//!
//! 它由 boot 直接引入（不由编排域起：`order: None`），起的第一个东西是编排域，之后只做一件事
//! ——照单发货。

use crate::program::{Program, Spot};
use env::ProgramKind;

pub static PROGRAM: Program = Program {
    name: "root",
    kind: ProgramKind::Supervisor,
    spot: Spot::Domain,
    scenes: &["root", "product"],
    // **同一个引导域起两景**：`root` 是验收镜像、`product` 是"真正要发出去的那一台"。
    entry: &["root", "product"],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};
