//! root::program — **引导域**（`prog-root`）的装配声明。
//!
//! 它由 boot 直接引入（**不在装配单上**：`after: None`），起的第一个东西是编排域，之后只做一件事
//! ——照单发货。

use crate::unit::{Demand, Identity, UnitFile, Relation};
use env::ProgramKind;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "root",
        space: ProgramKind::Supervisor,
        wanted_by: &["root", "product"],
        // **同一个引导域起两景**：`root` 是验收镜像、`product` 是"真正要发出去的那一台"。
        entry: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation::DEFAULT,
    demand: Demand::DEFAULT,
};
