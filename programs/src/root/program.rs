//! root::program — **引导域**（`prog-root`）的装配声明。
//!
//! 它由 boot 直接引入（**不在装配单上**：`deps: None`），起的第一个东西是编排域，之后只做一件事
//! ——照单发货。

use crate::program::{Demand, Identity, Program, Relation};
use env::ProgramKind;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "root",
        kind: ProgramKind::Supervisor,
        scenes: &["root", "product"],
        // **同一个引导域起两景**：`root` 是验收镜像、`product` 是"真正要发出去的那一台"。
        entry: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
