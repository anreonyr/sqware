//! operator::program — **持树者**（`prog-operator`）的装配声明。
//!
//! 它与其他每一台走同一条路：编排域按 `after` 算出的次序用 `mint` 起它。它**第一**起
//! （客人上树要它在）；起手把提示之路交给生我者（编排域）——**那一件事本身就是"它是持树者"的
//! 凭据**（`holds_tree` 那一格已退场，见 [`Relation`] 的头注）。

use crate::unit::{Demand, Died, Ending, Identity, UnitFile, Relation, Setup};

/// 它死在起手哪一步（板 / 树 / 收帧那一页）；名册与盟册的起手号同族不同格。
pub const E_TREE: Died = 10;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "operator",
        wanted_by: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&[]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // **答得动**：落完面（上树那一趟查回来验过）之后铸一枚刻 `READY` 的孔交给装配者
        // ——与三台驱动、设备账那两处**同一手**。被 `after` 指着的台必须说得出这一句。
        ..Demand::DEFAULT
    },
};
