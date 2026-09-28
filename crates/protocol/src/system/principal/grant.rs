//! principal::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//!
//! ```text
//!   Bind 定   Resolve 问   Derive 生   Adopt 领   Waive 弃   Sire 父   Heir 支   ← 七条原语
//!   Ask 问面：Resolve / Sire / Heir          Set 定面：Bind / Derive / Adopt / Waive
//! ```
//!
//! # 两面是**数出来**的，不是照条数切的
//!
//! 七条原语若照条数拆七格，其中三格**今天没有任何生产持有者**（`Adopt` / `Waive` / `Sire`
//! 只被测具选过）——那正是"报法字段"：造出来没人选。生产里的持有人只数得出三个：
//!
//! | 持有者 | 它实际叫哪几条 |
//! |---|---|
//! | 持树者的名册门（`operator/door.rs` 的 `Court`） | `Resolve` ＋ `Heir` |
//! | 盟册服务自己（`coalition/server.rs`） | `Resolve` |
//! | 装配者那一侧（`principal/bridge.rs` 的 `Roster`） | `Derive` ＋ `Bind` |
//!
//! ⇒ **三条"问"的 ＋ 四条"定"的**，一面一条。而"问的一条与定的那条同一面"是有后果的，且量得
//! 出来：**持树者手里那枚门牌做得出 `Adopt`**（把一条号领到自己底下——`probe-rule` 量的那一格：
//! `adopt(q)` 之后 `Trunk(p)` 立刻答 `Denied`），而它一辈子只用两条**读**的。这与 operator 拆
//! 七格是同一条理由（`find` 会交出能力 ⇒ 不与只读那几条合成一位）。
//!
//! # 面**就是那一枚门牌**（这一族与 operator 不同的一格）
//!
//! operator 那一族有会话：面在**会话入口的记号**上（`ask()`），服务端从它自己表里那枚问话孔
//! 读回来。principal **没有会话**——门牌自己就是那条路，而从前所有人往**同一枚**孔推帧，
//! 服务端因此**分不出面**。故这一族的面只能是**两枚门牌、两只孔**：服务端从**它自己表里哪一枚
//! 孔**收到，就是哪一面。那一枚门牌自己的记号仍然要，但只给**树 / 装配者查表**用——它们按
//! `(开者, 记号)` 两格在各自表里认出那一枚（`establish::find` / `operator::claim::find_face`）。
//!
//! **不上帧**：本文件一个字节都不进报文——线格式、动作码、失败域**一处未动**。
//!
//! # 位次与记号
//!
//! [`Grant::ALL`] 的**次序就是位次 1..=2**（第 i 位 = `ALL[i-1]`）——位次与枚举变体不各说各的。
//! 记号 = `"principal-" ＋ 面名`：它同时是**那一枚门牌 Pie 的记号**与**下面那一段入口路的
//! 末段名**（`/sys/principal/{ask,set}`），一个面名一处给。
//!
//! **照实记（两面各带自己的记号，没有一面留 `entry`）**：可以图省事让问面继续用那枚通用的
//! `board::ENTRY_MARK`、只给定面新记号——那样 `find_face(who)` 一个字都不用改。**被否**：
//! "没有记号的那一面是读面"是**位置即语义**（靠"哪一枚是默认的"分派），而这里恰恰是**判面**
//! 那一格；两面各带显式的记号之后，查表那一侧必须**说清自己要哪一面**（`find_face(who, mark)`）。
//!
//! # 机制那一半已经收进 [`crate::faces!`]（第二台落地之后）
//!
//! 本文件只交代**这一族自己的事实**：两面各叫什么、哪条线上码落哪一面、记号词根、还要与谁
//! 不相撞。位次怎么算、记号怎么拼、认面怎么扫、那组断言怎么排——**共用的那一份在
//! [`crate::system::faces`]**（"为什么到第二台才抽"那一份文件头里有量出来的数）。

use env::Mark;

use super::frame::{BACK, Wire};

crate::faces! {
    /// **一条权柄边界**：一枚 = 一面。两位，位次 1..=2。
    pub enum Grant {
        /// **问面**：`Resolve` / `Sire` / `Heir`——只读，不改任何一个字。
        Ask => "ask",
        /// **定面**：`Bind` / `Derive` / `Adopt` / `Waive`——改身份或改谱系。
        Set => "set",
    }
    stem: "principal-",
    name_max: 3,
    wire_ty: Wire,
    wire: {
        // 七条线上码逐条说它落哪一面（**不并 `|`**：见 `faces!` 那份照实记）。
        Wire::Resolve(_) => Ask,
        Wire::Sire(_) => Ask,
        Wire::Heir(_, _) => Ask,
        Wire::Bind(_, _) => Set,
        Wire::Derive(_) => Set,
        Wire::Adopt(_) => Set,
        Wire::Waive => Set,
    }
    distinct: [
        BACK,
        // 另外几族的记号按**字面量**给（不跨族 `use`）：入口通用那一枚、提示那一枚、
        // 板那一枚、控制面那一枚、树那七位里的第一位。
        Mark::of("entry"),
        Mark::of("tip"),
        Mark::of("board-ask"),
        Mark::of("control-ask"),
        Mark::of("operator-ask-part"),
    ],
}
