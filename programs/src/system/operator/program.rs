//! operator::program — **持树者**（`prog-operator`）的装配声明。
//!
//! 它与其他每一台走同一条路：编排域按 `order: Some(0)` 用 `mint` 起它。它**第一**起
//! （客人上树要它在），且 `holds_tree: true`——起来时把提示之路交给生我者（编排域）。

use crate::program::{Demand, Died, Ending, Identity, Program, Relation};

/// 它死在起手哪一步（板 / 树 / 收帧那一页）；名册与盟册的起手号同族不同格。
pub const E_TREE: Died = 10;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "operator",
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        order: Some(0),
        ending: Some(Ending::Resident),
        presence: true,
        bind: true,
        holds_tree: true,
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_TREE,
        ..Demand::DEFAULT
    },
};
