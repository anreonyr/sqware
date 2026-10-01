//! coalition::program — **盟册**（`prog-coalition`）的装配声明。
//!
//! 结盟服务：答"这一位在那枚盟里吗"。它是持树者的**第二双眼睛**（`eyes: Some(Eyes::League)`）。

use crate::program::{Demand, Died, Ending, Identity, Program, Relation};
use env::wire::Eyes;

/// 它死在起手哪一步（那只组 / 找名册那份门牌）。
pub const E_COALITION: Died = 16;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "coalition",
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        order: Some(2),
        ending: Some(Ending::Resident),
        presence: true,
        operator: true,
        bind: true,
        eyes: Some(Eyes::League),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_COALITION,
        ..Demand::DEFAULT
    },
};
