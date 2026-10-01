//! probe_control — **这一台**的装配声明（身子在本目录的 `main.rs`；"哪几台进哪张镜像"见
//! [`crate::unit::catalog`] 的 `PROGRAMS`）。

use crate::unit::{Demand, Ending, Identity, Relation, SCENE, UnitFile};

/// **控制面的真客人**：从树上找 **`/svc/sys/control/state`**（问面），问一句 control 的话；
/// 另取那三面各期望被拒（带规矩），并拿问面发写、期望判面拒。
/// task-4 那条挂载路挂出过一块**查得到、取不回**的门牌（铸入口的是一枚一次性边沿线程，
/// 它一收尾，持树者表里那枚副本就被内核的派生链级联摘掉）。这一台量的正是那件事的反面：
/// **在另一个域里**照 principal / coalition 逐字同形的路找上门、把门牌取回来、问一句话。
/// 判据两条（`programs/src/harness/probe/probe_control/main.rs`）：表外那个名字答 `Unknown`、本台自己答得出一个
/// 生命阶段——`Bad`（这一趟没走到对面）在两条里都是红。
/// **它排在最后**（`after` 里那条 [`SCENE`] 边）：那一面是在**整表起完**之后才挂上树的
/// （`Assembly::mount_control`，由 `system/main.rs` 的相四叫）——那**不是一个台**，图里本来
/// 没有它的落点，故写成"等装配那一趟走完"那条边（名字 [`SCENE`]，次序由
pub static PROBE_CONTROL: UnitFile = UnitFile {
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
