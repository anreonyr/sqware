//! principal::program — **名册**（`prog-principal`）的装配声明。
//!
//! 身份服务：答"这一位此刻代表谁"与"在不在他那一支里"。它是持树者的**第一双眼睛**
//! （`eyes: Some(Eyes::Roster)`）——那一格不是靠名字认的。

use crate::program::{Demand, Died, Ending, Identity, Program, Relation, Setup};
use env::wire::Eyes;

/// 它死在起手哪一步（落门牌 / 立两张表 / 回查）。
pub const E_PRINCIPAL: Died = 14;

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "principal",
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        deps: Some(&["operator"]),
        ending: Some(Ending::Resident),
        presence: true,
        operator: true,
        bind: true,
        eyes: Some(Eyes::Roster),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_PRINCIPAL,
        // **答得动**：落完面（上树那一趟查回来验过）之后铸一枚刻 `READY` 的孔交给装配者
        // ——与三台驱动、设备账那两处**同一手**。被 `deps` 指着的台必须说得出这一句。
        setup: &[Setup::Ready(crate::program::READY)],
        ..Demand::DEFAULT
    },
};
