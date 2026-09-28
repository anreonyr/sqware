//! operator::grant —— **操作面那一维**：一枚 `Grant` = 一枚操作。
//!
//! ```text
//!   part 分   land 落   find 寻   trim 剪   list 列   seek 译   name 名      ← 七条原语
//!   Face 一面持树者（全操作面）   Grant 一柄授面的权（**一枚 = 一枚操作**）
//! ```
//!
//! 三位一体一句话：**来源 = 树上那一格，判别 = 会话说的是哪一位**。故
//!
//! - **不可伪造**：请求里没有"我是哪一位"这一格；面在**会话入口的记号**上（`ask()`），
//!   服务端从**它自己表里那枚问话孔**读回（[`grant_of`]）⇒ 客户端无从自称；
//! - **不替代** [`Permit`](super::Permit) 与砖上那 `owner` 一格：它只判"这一位许不许这一类"，
//!   过了之后原来那两道门照走（`land` 仍是"面 ✓ ＋ 那一格归我 ✓"两层）；
//! - **不上帧**：本文件一个字节都不进报文——线格式、动作码、失败域**一处未动**。
//!
//! # 位次与记号
//!
//! [`Grant::ALL`] 的**次序就是位次 1..=7**（第 i 位 = `ALL[i-1]`），故位次与枚举变体
//! 不会各说各的。记号 = `"operator-ask-" + 面名`：它同时是**那一格入口 Pie 的记号**与
//! **那一面会话说问话孔的记号**（同一个面名，一处给）。
//!
//! **照实记（`0` 那个哨兵退场了）**：从前的 [`Grant::of_wire`] 有一格 `_ => 0`——"表外的动作"
//! 落 `0`，而 `0` 不是任何一位，故它从不参与判面。今天那一条 `match` **穷尽 `Wire`**（七条线上
//! 动作一条不漏，漏了编不过）：表外的**动作码**在解帧那一侧就答 `None` / `BAD`，**走不到这里**
//! ⇒ 那一格哨兵连同"靠 `0` 区分"这件事一起撤了。
//!
//! # 名字为什么是 `Grant`
//!
//! 一名两读，一眼兼两意：**这一面的「一位」**（位次）与**授出去的「一柄权」**。故不必为
//! 客户端与服务端各造一个词——客户端那具柄叫 [`Rein`](super::client::Rein)（与 `Face` 对偶），
//! 服务端判的那一句就叫 [`Grant::at`]。
//!
//! # 机制那一半已经收进 [`crate::faces!`]（第二台落地之后）
//!
//! 本文件今天只交代**这一族自己的事实**：七位各叫什么、哪条线上码落哪一位、记号词根、还要与谁
//! 不相撞。位次怎么算、记号怎么拼、认面怎么扫、那组断言怎么排——**共用的那一份在
//! [`crate::system::faces`]**（"为什么到第二台才抽"那一份文件头里有量出来的数）。

use env::Mark;

use super::frame::Wire;
use super::{ASK_MARK, TIP_MARK};

crate::faces! {
    /// **一柄授面的权**：一枚 = 一枚操作。七位，位次 1..=7。
    pub enum Grant {
        /// `part` 分。
        Part => "part",
        /// `land` 落。
        Land => "land",
        /// `find` 寻。
        Find => "find",
        /// `trim` 剪。
        Trim => "trim",
        /// `list` 列。
        List => "list",
        /// `seek` 译。
        Seek => "seek",
        /// `name` 名。
        Name => "name",
    }
    stem: "operator-ask-",
    name_max: 4,
    wire_ty: Wire,
    wire: {
        Wire::Part { .. } => Part,
        Wire::Land { .. } => Land,
        Wire::Find(_) => Find,
        Wire::Trim(_) => Trim,
        Wire::List(_) => List,
        // `Road` 就是 `seek`（路那一形只到这一格，见 [`Req`](super::Req)）。
        Wire::Road(_, _) => Seek,
        Wire::Name(_) => Name,
    }
    distinct: [ASK_MARK, TIP_MARK, Mark::of("board-ask")],
}
