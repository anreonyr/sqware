//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// 验证旧操作路径移除、写请求准入与独立会话。
pub static PROBE_OPERATOR_GATE: UnitFile = UnitFile {
    publication: &[],
    identity: Identity {
        name: "probe-operator-gate",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
