//! operator::program — **持树者**（`prog-operator`）的装配声明。
//!
//! 它与其他每一台走同一条路：编排域按 `order: Some(0)` 用 `mint` 起它。它**第一**起
//! （客人上树要它在），且 `holds_tree: true`——起来时把提示之路交给生我者（编排域）。

use crate::program::{Demand, Died, Identity, Origin, Program, Relation, Spot};
use env::ProgramKind;

/// 它死在起手哪一步（板 / 树 / 收帧那一页）；名册与盟册的起手号同族不同格。
pub const E_TREE: Died = 10;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "operator",
        kind: ProgramKind::User,
        spot: Spot::Service,
        scenes: &["root", "product"],
        entry: &[],
    },
    relation: Relation {
        order: Some(0),
        presence: true,
        operator: false,
        bind: true,
        holds_tree: true,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_TREE,
        setup: &[],
    },
};
