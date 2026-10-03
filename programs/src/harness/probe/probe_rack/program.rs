//! probe-rack 的装配声明（身子在本目录 `main.rs`；"哪几台进哪张镜像"见 `unit/catalog.rs`）。
//!
//! **一具架的队列语义与唤醒协议**（单域、确定性）：满了怎么丢、读端怎么走、铃为什么不会空转。
//! 次序：它不跟谁说话，仍排在树那条路之后起（与其它探针同一条，避开装配那一趟）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROBE_RACK: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: None,
    #[cfg(target_arch = "riscv64")]
    prepare: None,
    identity: Identity {
        name: "probe-rack",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
