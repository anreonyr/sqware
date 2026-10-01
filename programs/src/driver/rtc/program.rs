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
        // 于是那份依赖一直没有凭据。撤板那一刀（把板那条会话当节拍的那点偶然先后拿掉）**逼出了
        // 这一条**，`router` 有 `Setup::Ready` 凭据，故这一条边说得出"要它答得动"。
        //
        // **照实记（那条红是"本来就有的抖"：A/B 两面都出得来，这一条边不是它的解）**：
        // `rtc` ＋ `line` 那一红（`exit tid=… reason=0xc note: line` → 装配者报 `system: assemble`）
        // 在**不带**撤板那一刀的 A 面也出得来：
        //   · A 面（`33d69c8`、手工 `boot.nu`、release、喂 `exit`）三跑：**16 / 14（红，同一签名）/ 16**；
        //   · B 面（带撤板那一刀）三跑：**16 / 14 / 14**；debug 档两跑：**16 / 10**。
        // n=3 ⇒ **两侧分不开**（上一轮曾把它判成"那一刀破了它"：B 面连红三次而 A 面那几跑恰好全绿，
        // 那是样本不足的误判——**收回**）。**这一条边的价值与那条红分开记**：边是真缺的；红是这台
        // 机器早就有的一种抖，成因待查（判据：本台在 `line` 那一步收到"问不动"的答）。
        after: Some(&["operator", "hub", "router"]),
        ending: Some(Ending::Resident),
        ..Relation::DEFAULT
    },
    demand: Demand {
        died: E_RTC,
        // **起手最后一步（落面）之后才交**：这一格就是「答得动」的凭据。
        setup: &[Setup::Ready],
        ..Demand::DEFAULT
    },
};
