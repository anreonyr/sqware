//! coalition::program — **盟册**（`prog-coalition`）的装配声明。
//!
//! 结盟服务：答"这一位在那枚盟里吗"。它是持树者的**第二双眼睛**（`eyes: Some(Eyes::League)`）。

use crate::program::{Died, Program, Spot};
use env::ProgramKind;
use env::wire::Eyes;

/// 它死在起手哪一步（那只组 / 找名册那份门牌）。
pub const E_COALITION: Died = 16;

pub static PROGRAM: Program = Program {
    name: "coalition",
    kind: ProgramKind::User,
    spot: Spot::Service,
    scenes: &["root", "product"],
    entry: &[],
    order: Some(2),
    board: true,
    operator: true,
    bind: true,
    holds_tree: false,
    eyes: Some(Eyes::League),
    died: E_COALITION,
    setup: &[],
};
