//! rtc::program — **实时钟驱动**（`prog-rtc`）的装配声明。
//!
//! **U 态**：持有 `rtc@101000`（11 号线），武装闹钟、到点自己拉线；客人定的闹钟到点就清掉
//! 那一格、把"那一声"推回去。它是"抽象等第二个实例"的那个第二例。

use crate::program::{Demand, Died, Identity, Origin, Program, Relation, Setup, Spot};
use env::supply::{Kind, Need, class_block};
use env::{Access, Policy, ProgramKind};

/// 它死在起手 / 常驻哪一步。
pub const E_RTC: Died = 12;

/// 实时钟驱动要的那一枚：**那一台 `google,goldfish-rtc`**。
pub const RTC_WANTS: &[Need] = &[Need::class(
    class_block("google,goldfish-rtc"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];

pub static PROGRAM: Program = Program {
    identity: Identity {
        name: "rtc",
        kind: ProgramKind::User,
        spot: Spot::Service,
        scenes: &["root", "product"],
        entry: &[],
    },
    relation: Relation {
        order: Some(5),
        presence: true,
        operator: true,
        bind: true,
        holds_tree: false,
        eyes: None,
    },
    demand: Demand {
        origin: Origin::Initrd,
        died: E_RTC,
        setup: &[Setup::Need(RTC_WANTS[0]), Setup::Channel("records")],
    },
};
