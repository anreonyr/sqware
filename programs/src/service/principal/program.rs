//! principal::program — **名册**（`prog-principal`）的装配声明。
//!
//! 身份服务：答"这一位此刻代表谁"与"在不在他那一支里"。它是持树者的**第一双眼睛**：它起手把
//! `Grant::Ask` 那一枚门牌交给持树者，**持树者按记号就认得出它**（`eyes` 那一格已退场，
//! 见 [`Relation`] 的头注）；而装配者那一侧只剩一件真事——认下它交来的 `Grant::Set`
//! （`principal::bridge::adopt_roster`）。

use crate::unit::{Demand, Died, Ending, Identity, UnitFile, Relation, Setup};

/// 它死在起手哪一步（落门牌 / 立两张表 / 回查）。
pub const E_PRINCIPAL: Died = 14;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "principal",
        wanted_by: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_PRINCIPAL,
        // **答得动**：落完面（上树那一趟查回来验过）之后铸一枚刻 `READY` 的孔交给装配者
        // ——与三台驱动、设备账那两处**同一手**。被 `after` 指着的台必须说得出这一句。
        supply: &[Setup::Ready],
        ..Demand::DEFAULT
    },
};
