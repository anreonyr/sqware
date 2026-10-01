//! 盟册（prog-coalition）的装配声明。
//! 结盟服务：答"这一位在那枚盟里吗"。它是持树者的**第二双眼睛**——它起手把 Grant::Ask 那一枚

use crate::unit::{Demand, Died, Ending, Identity, Relation, UnitFile};

/// 它死在起手哪一步（那只组 / 找名册那份门牌）
pub const E_COALITION: Died = 16;

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "coalition",
        wanted_by: &["accept", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "principal"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        // ——与三台驱动、设备账那两处**同一手**。被 `after` 指着的台必须说得出这一句。
        ..Demand::DEFAULT
    },
};
