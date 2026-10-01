//! 设备账（prog-hub）的装配声明。
//! **U 态**：它不碰寄存器、不碰中断——只读设备树（把坐标补成"名 / 类 / 线"）、立账、落
//! `/dev/<类>/<名>`、答三面。故它与 `operator` / `principal` / `coalition` 同一档，不需要特权。

use crate::unit::{Demand, Died, Ending, Identity, Relation, Setup, UnitFile};

pub const E_HUB: Died = 28;

pub const CHANNEL: &str = "hub";

/// **末尾**铸一枚刻它的孔交回装配者，那一刻它才继续往下起别人。
pub const READY: &str = "hub-ready";

pub static PROGRAM: UnitFile = UnitFile {
    identity: Identity {
        name: "hub",
        wanted_by: &["accept", "product"],
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
