//! probe-rack-mount 的装配声明（身子在本目录 `main.rs`）。
//!
//! **铺场那一侧**：开两具架、落**两枚**砖、把 A 写满、响 `Ready`，并在收尾封印自己那一枚页（验树剔死那一格）。
//! 它要给 `Ready` 凭据——客人那一台的 `after` 指着它（`UnitFile::supply` 按这条边推出来）。

use crate::unit::{Demand, Ending, Identity, Relation, UnitFile};

pub static PROBE_RACK_MOUNT: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: Some(crate::harness::probe::hierarchy::publication),
    #[cfg(target_arch = "riscv64")]
    prepare: None,
    identity: Identity {
        name: "probe-rack-mount",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator"]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
