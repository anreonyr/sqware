//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! crate::unit::catalog 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 主体：身份服务的第一位真客人。
pub static SUBJECT: UnitFile = UnitFile {
    identity: Identity {
        name: "subject",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "principal"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
