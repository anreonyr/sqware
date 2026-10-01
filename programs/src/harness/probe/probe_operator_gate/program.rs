//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

/// **操作面的正证客人（全操作面那一半）**：拿控制面会话把 `/svc/sys/operator` 与它底下那几格看
/// 一眼、取回 `/svc/sys/operator/land` 那一枚入口、再把试验场（**根**底下两格归属不同的砖）铺好
/// 给下一位客人
/// 故它一上来就看得见；**那七段名字的读数归树自己**
pub static PROBE_OPERATOR_GATE: UnitFile = UnitFile {
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
