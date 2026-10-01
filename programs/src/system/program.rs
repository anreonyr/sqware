//! program::system — **编排域**（`prog-system`）自己的装配声明。
//!
//! 本文件**不在 `system` 那棵模块树里**（那棵树拖着 runtime / protocol，`crates/image` 进不去）
//! ——它由 `programs/src/program.rs` 的 `#[path]` 拉进注册表，路径是 `crate::program::system`。
//!
//! 它是 boot 之后**唯一**起服务的地方，自己由引导域起（**不在装配单上**：`deps: None`）。整台机器的服务都由它
//! 按各台声明里的 `deps` **算出来的次序**依次起（`program::order_scene`）。

use crate::program::{Demand, Died, Identity, Program, Relation};
use env::ProgramKind;

/// 起手第一步没成：与引导域那条会话没搭上。
pub const E_BOOT: Died = 1;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "system",
        space: ProgramKind::Supervisor,
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
