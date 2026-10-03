//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static GUEST: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: None,
    #[cfg(target_arch = "riscv64")]
    prepare: None,
    identity: Identity {
        name: "guest",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "router"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
