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
        // **`router` 这一条边是补上的**（照实记：缺它的时候，本台靠**位次**站在路由者后面——
        // 而它起手那一步 `Context::line` 恰恰**要问路由者**（`/svc/drv/router` 那一格 + 请它占线，
        // 见 `driver/context.rs::line`）。**"位置即语义"**：两条边之间没有话，只有排序的先后，
        // 于是那份依赖一直没有凭据。撤板那一刀把板那条会话当节拍的那点偶然先后拿掉之后，本台
        // 当场死在 `line`（release 档实测：`rtc` ＋ `line`；debug 档不显）——那一刀**逼出了这一条**。
        // `router` 有 `Setup::Ready` 凭据，故这一条边说得出"要它答得动"。
        deps: Some(&["operator", "hub", "router"]),
        ending: Some(Ending::Resident),
        presence: true,
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_RTC,
        // **起手最后一步（落面）之后才交**：这一格就是「答得动」的凭据。
        setup: &[Setup::Ready],
        ..Demand::DEFAULT
    },
};
