//! sleeper — **这一台**的装配声明（身子在本目录的 `main.rs`；"哪几台进哪张镜像"见
//! [`crate::unit::catalog`] 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 客人：`/svc/drv/rtc` 那面服务的第一位用家。
pub static SLEEPER: UnitFile = UnitFile {
    identity: Identity {
        name: "sleeper",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "rtc"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
