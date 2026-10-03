//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Demand, Ending, Identity, Relation, SCENE, UnitFile};

/// **控制面的真客人**：从树上找 **`/svc/sys/control/state`**（问面），问一句 control 的话
/// 另取那三面各期望被拒（带规矩），并拿问面发写、期望判面拒
/// task-4 那条挂载路挂出过一块**查得到、取不回**的门牌（铸入口的是一枚一次性边沿线程
/// 它一收尾，持树者表里那枚副本就被内核的派生链级联摘掉）。这一台量的正是那件事的反面
/// **在另一个域里**照 Identity 同形的路找上门、把门牌取回来、问一句话
/// 判据两条（`programs/src/harness/probe/probe_control/main.rs`）：表外那个名字答 `Unknown`、本台自己答得出一个
/// （Assembly::mount_control，由 `system/main.rs` 的相四叫）——那**不是一个台**，图里本来
pub static PROBE_CONTROL: UnitFile = UnitFile {
    #[cfg(target_arch = "riscv64")]
    publication: None,
    #[cfg(target_arch = "riscv64")]
    prepare: None,
    identity: Identity {
        name: "probe-control",
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", SCENE]),
        restart: Some(Ending::Transient),
        ..Relation::DEFAULT
    },
    demand: Demand { ..Demand::DEFAULT },
};
