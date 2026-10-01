//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 盟友：结盟服务的第一位真客人
pub static MEMBER: UnitFile = UnitFile {
    identity: Identity {
        name: "member",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "coalition", "principal"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
