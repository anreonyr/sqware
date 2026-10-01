//! coalition::program — **盟册**（`prog-coalition`）的装配声明。
//!
//! 结盟服务：答"这一位在那枚盟里吗"。它是持树者的**第二双眼睛**（`eyes: Some(Eyes::League)`）。

use crate::program::{Demand, Died, Ending, Identity, Program, Relation, Setup};
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
        deps: Some(&["operator", "principal"]),
        ending: Some(Ending::Resident),
        presence: true,
        bind: true,
        eyes: Some(Eyes::League),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_COALITION,
        // **答得动**：落完面（上树那一趟查回来验过）之后铸一枚刻 `READY` 的孔交给装配者
        // ——与三台驱动、设备账那两处**同一手**。被 `deps` 指着的台必须说得出这一句。
        setup: &[Setup::Ready(crate::program::READY)],
        ..Demand::DEFAULT
    },
};
