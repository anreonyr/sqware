//! hub::program — **设备账**（`prog-hub`）的装配声明。
//!
//! **U 态**：它不碰寄存器、不碰中断——只读设备树（把坐标补成"名 / 类 / 线"）、立账、落
//! `/dev/<类>/<名>`、答三面。故它与 `operator` / `principal` / `coalition` 同一档，不需要特权。
//!
//! **它必须排在驱动之前**（`order: 3`）：驱动起手第一件事是认领（它要设备账在树上、
//! 也要盟册在），而那一问要 hub 已经在树上（[`driver/mod.rs`](crate::driver) 那条"先起的先
//! 就绪"）。⇒ 本轮把原次序 3..=21 整段后移一位（**相对次序一格不变**），hub 占 3。
//!
//! **它不上板**（`presence: false`）：hub 死了，已经认领过设备的驱动手里有副本、照跑；未认领的
//! 没人发得出——那是一条已知边界（见 [`crate::service::hub`] 的头注），不是靠板兜的事。
//! **它接树**（**推出来的**：`after` 里那条指向 `operator` 的边——`operator: true` 那一格已退场，
//! 见 [`Relation`] 的头注）：它自己要把三枚面挂 `/svc/hub`、把设备格落 `/dev`。
//!
//! **它起手要"整机物料"**（[`Setup::Machine`]）：装配者按机器自述枚举全机、逐段向引导域领、
//! 从这一条通道把记录推给它——**这不是"要一枚门闩"，是"要这一台机器"**（hub 是唯一一个
//! 手上必须握着每一台设备那一份的域：它的工作就是把它再授出去）。
//!
//! **那一个字面量 `"hub"`**：它与 [`crate::service::hub`] 在树上的那一段名字同一个词
//! （`protocol::driver::hub` 那一族在树上的那一段名字）——本文件**只许引 `env`**（宿主打包要读它，见
//! [`crate::unit`] 的头注），故那个词只能在这儿写第二遍。对不上时 hub 收不到物料
//! （当场 `hub: no machine`），不会静默跑起来。

use crate::unit::{Demand, Died, Ending, Identity, UnitFile, Relation, Setup};

/// 它死在起手哪一步（读完机器自述、立账那一趟）。
pub const E_HUB: Died = 28;

/// **收物料那条通道的名字**——两端同一个（本域 `establish::endpoint` 铸的就是刻它的孔）。
///
/// **照实记（那个词只能写在这儿，而它就是树上的那一段名字）**：本文件**只许引 `env`**
/// （宿主打包要读它，见 [`crate::unit`] 的头注），故它写不出
/// `protocol::driver::hub::NAME` 那条引用——而两者**本来就是同一个词**（"hub 那条路"与
/// "hub 那条通道"是同一位的两条边，不另起名）。**装配那两端照这一格对齐**：本域铸孔用
/// [`CHANNEL`]，装配者 [`connect`](crate::system::control::connect) 认的也是它；对不上时
/// 本域收不到物料——当场 `hub: no machine`，不会静默跑起来。
///
/// **照实记（协议那一侧那一格退了）**：`protocol::driver::hub` 里曾经也有一枚同名常量，
/// 而它一个代码读者都没有（读的是这一格）⇒ 死格退掉，这一格是**代码里唯一一处**。
pub const CHANNEL: &str = "hub";

/// **"我起完了"那条通道的名字**（见 [`Setup::Machine`] 的 `ready` 那一格）：本域在起手
/// **末尾**铸一枚刻它的孔交回装配者，那一刻它才继续往下起别人。
pub const READY: &str = "hub-ready";

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "hub",
        wanted_by: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        after: Some(&["operator", "coalition"]),
        restart: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        supply: &[Setup::Machine {
            load: CHANNEL,
            ready: READY,
        }],
        ..Demand::DEFAULT
    },
};
