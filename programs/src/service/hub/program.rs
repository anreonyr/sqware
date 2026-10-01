//! hub::program — **设备账**（`prog-hub`）的装配声明。
//! **U 态**：它不碰寄存器、不碰中断——只读设备树（把坐标补成"名 / 类 / 线"）、立账、落
//! `/dev/<类>/<名>`、答三面。故它与 `operator` / `principal` / `coalition` 同一档，不需要特权。

use crate::unit::{Demand, Died, Ending, Identity, Relation, Setup, UnitFile};

/// 它死在起手哪一步（读完机器自述、立账那一趟）。
pub const E_HUB: Died = 28;

/// **收物料那条通道的名字**——两端同一个（本域 `establish::endpoint` 铸的就是刻它的孔）。
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
