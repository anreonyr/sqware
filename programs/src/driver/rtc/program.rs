//! rtc::program — **实时钟驱动**（`prog-rtc`）的装配声明。
//!
//! **U 态**：持有 `rtc@101000`（11 号线），武装闹钟、到点自己拉线；客人定的闹钟到点就清掉
//! 那一格、把"那一声"推回去。它是"抽象等第二个实例"的那个第二例。

use crate::program::{Demand, Died, Ending, Identity, Program, Relation, Setup};

/// 它死在起手 / 常驻哪一步。
pub const E_RTC: Died = 12;

// **照实记（"要的那一枚"搬回本域）**：同 `uart` 那一份——需求单回了本域自己的模块
// （`driver/rtc/main.rs` 的 `ASK`）。

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "rtc",
        scenes: &["root", "product"],
        ..Identity::DEFAULT
    },
    relation: Relation {
        deps: Some(&["operator", "hub"]),
        ending: Some(Ending::Resident),
        presence: true,
        operator: true,
        bind: true,
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_RTC,
        // **起手最后一步（落面）之后才交**：这一格就是「答得动」的凭据。
        setup: &[Setup::Ready(crate::program::READY)],
        ..Demand::DEFAULT
    },
};
