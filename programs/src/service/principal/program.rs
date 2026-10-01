//! 名册（prog-principal）的装配声明。
//! 身份服务：答"这一位此刻代表谁"与"在不在他那一支里"。它是持树者的**第一双眼睛**：它起手把

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手哪一步（落门牌 / 立两张表 / 回查）
pub const E_PRINCIPAL: Died = 14;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "principal",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // ——与三台驱动、设备账那两处**同一手**。被 `after` 指着的台必须说得出这一句。
        ..Demand::DEFAULT
    },
};
