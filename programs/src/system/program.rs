//! program::system — **编排域**（`prog-system`）自己的装配声明。
//!
//! 本文件**不在 `system` 那棵模块树里**（那棵树拖着 runtime / protocol，`crates/image` 进不去）
//! ——它由 `programs/src/program.rs` 的 `#[path]` 拉进注册表，路径是 `crate::program::system`。
//!
//! 它是 boot 之后**唯一**起服务的地方，自己由引导域起（`order: None`）。整台机器的服务都由它
//! 按 [`PROGRAMS`](crate::program::PROGRAMS) 里各台的 `order` 依次起。

use crate::program::{Died, Program, Spot};
use env::ProgramKind;

/// 起手第一步没成：与引导域那条会话没搭上。
pub const E_BOOT: Died = 1;

pub static PROGRAM: Program = Program {
    name: "system",
    kind: ProgramKind::Supervisor,
    spot: Spot::Domain,
    scenes: &["root", "product"],
    entry: &[],
    order: None,
    board: false,
    operator: false,
    bind: false,
    holds_tree: false,
    eyes: None,
    died: env::EXIT_OK,
    setup: &[],
};
