//! coalition::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//!
//! ```text
//!   Found 立   Enter 入   Leave 出   Admit 代报名   Amid 问   Band 点名册   Bloc 问此人在哪些盟   ← 七条原语
//!   Ask 问面：Amid / Band / Bloc             Set 定面：Found / Enter / Leave / Admit
//! ```
//!
//! # 两面是**数出来**的，不是照条数切的（第三家）
//!
//! 七条原语若照条数拆七格，只有一格有生产持有者——那是"报法字段"造出来的七倍。生产里的持有
//! 人数得出**一个**：
//!
//! | 持有者 | 它实际叫哪几条 |
//! |---|---|
//! | 持树者那枚盟册门牌（`operator/door.rs` 的 `Court`） | **`Amid` 一条** |
//!
//! 而其余五条（`Band` / `Bloc` / `Found` / `Enter` / `Leave`）在生产里**零调用者**（只有
//! `member` / `probe_rule` 两台测具握着）。⇒ 拆两面：**三条"问"的住 [`Grant::Ask`]**（问盟籍、
//! 点名册、问此人在哪些盟），**四条"定"的住 [`Grant::Set`]**（立盟、入、出、代报名）。
//!
//! **病与名册那一族同款，且更刺**：那枚交给持树者的门牌**做得出 `Found`**（立一枚盟）、
//! **做得出 `Enter` / `Leave`**（改盟籍）——而它一辈子只叫 `Amid`（"这一位在那枚盟里吗"）。
//! 名册那一族是同一刀（`Ask` / `Set`），那一族的正证与量法见
//! [`principal::grant`](crate::system::principal::grant)。
//!
//! **照实记（`Set` 面的第一位真持有者来了：设备账那一台）**：这一面开的时候写的理由是
//! "不把三条写的划出去，持树者那枚门牌就继续做得出 `Found`"——而当时**生产里没有那样的客人**
//! （谁立盟谁入盟，只有测具在做）。这一刀把第一位客人带来了：**hub** 立盟（每类一枚）
//! 并 `admit` 代四位驱动报名（`protocol::driver::hub` 的 `bond`）。`Set` 从"为一条真话开的"
//! 变成"有持有者的一条路"。
//!
//! # 面**就是那一枚门牌**（同 principal；两家都没有会话）
//!
//! 门牌自己就是那条路，所有人往它推帧。故面是**两枚门牌、两只孔**：服务端从**它自己表里哪一枚
//! 孔**收到就是哪一面，而"这一问属不属于这一面"由 [`Grant::of_wire`] 当场对一次（对不上答
//! [`DENIED`](super::DENIED)）。那一枚门牌自己的记号只给**树查表**用——它按 `(开者, 记号)`
//! 两格在各自表里认出那一枚（`operator::claim::find_face`）。
//!
//! **线上帧一个字节未动**（`CoordFrame` 照旧一枚号一次：树要的那三条读的住在同一面上）；唯一动
//! 的是失败域多了一格 [`Fail::Denied`](super::Fail::Denied)——那一格是这一刀添的，见它自己的注。
//!
//! # 机制那一半在 [`crate::faces!`]
//!
//! 本文件只交代**这一族自己的事实**：两面各叫什么、哪条线上码落哪一面、记号词根。**"与别族的
//! 记号不相撞"那一半不在这里**：它只有一处——[`crate::system`] 的全族总表（照实记见
//! [`crate::faces!`]）。

use super::frame::Wire;

crate::faces! {
    /// **一条权柄边界**：一枚 = 一面。两位，位次 1..=2。
    pub enum Grant {
        /// **问面**：`Amid` / `Band` / `Bloc`——只读，不改盟册一个字。
        Ask => "ask",
        /// **定面**：`Found` / `Enter` / `Leave` / `Admit`——立盟、入、出、代报名。
        Set => "set",
    }
    stem: "coalition-",
    name_max: 4,
    wire_ty: Wire,
    wire: {
        // 七条线上码逐条说它落哪一面（**不并 `|`**：见 `faces!` 那份照实记）。
        Wire::Amid(_, _) => Ask,
        Wire::Band(_, _) => Ask,
        Wire::Bloc(_, _) => Ask,
        Wire::Found => Set,
        Wire::Enter(_) => Set,
        Wire::Leave(_) => Set,
        Wire::Admit(_, _) => Set,
    }
}
