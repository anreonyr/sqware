//! coalition::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//! ```text
//!   Found 立   Enter 入   Leave 出   Admit 代报名   Amid 问   Band 点名册   Bloc 问此人在哪些盟   ← 七条原语
//!   Ask 问面：Amid / Band / Bloc             Set 定面：Found / Enter / Leave / Admit
//! ```
//! # 两面是**数出来**的，不是照条数切的（第三家）
//! 七条原语若照条数拆七格，只有一格有生产持有者——那是"报法字段"造出来的七倍。生产里的持有
//! 人数得出**一个**：
//! | 持有者 | 它实际叫哪几条 |
//! |---|---|
//! | 持树者那枚盟册门牌（`operator/door.rs` 的 `Court`） | **`Amid` 一条** |
//! 而其余五条（`Band` / `Bloc` / `Found` / `Enter` / `Leave`）在生产里**零调用者**（只有
//! `member` / `probe_rule` 两台测具握着）。⇒ 拆两面：**三条"问"的住 [`Grant::Ask`]**（问盟籍、
//! 点名册、问此人在哪些盟），**四条"定"的住 [`Grant::Set`]**（立盟、入、出、代报名）。
//! **病与名册那一族同款，且更刺**：那枚交给持树者的门牌**做得出 `Found`**（立一枚盟）、
//! **做得出 `Enter` / `Leave`**（改盟籍）——而它一辈子只叫 `Amid`（"这一位在那枚盟里吗"）。
//! 名册那一族是同一刀（`Ask` / `Set`），那一族的正证与量法见
//! [`principal::grant`](crate::service::principal::grant)。

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
        Wire::Amid(_, _) => Ask,
        Wire::Band(_, _) => Ask,
        Wire::Bloc(_, _) => Ask,
        Wire::Found => Set,
        Wire::Enter(_) => Set,
        Wire::Leave(_) => Set,
        Wire::Admit(_, _) => Set,
    }
}
