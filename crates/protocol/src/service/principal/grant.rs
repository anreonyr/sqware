//! principal::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//! ```text
//!   Bind 定   Resolve 问   Derive 生   Adopt 领   Waive 弃   Sire 父   Heir 支   ← 七条原语
//!   Ask 问面：Resolve / Sire / Heir          Set 定面：Bind / Derive / Adopt / Waive
//! ```
//! # 两面是**数出来**的，不是照条数切的
//! 七条原语若照条数拆七格，其中三格**今天没有任何生产持有者**（`Adopt` / `Waive` / `Sire`
//! 只被测具选过）——那正是"报法字段"：造出来没人选。生产里的持有人只数得出三个：
//! | 持有者 | 它实际叫哪几条 |
//! |---|---|
//! | 持树者的名册门（`operator/door.rs` 的 `Court`） | `Resolve` ＋ `Heir` |
//! | 盟册服务自己（`coalition/server.rs`） | `Resolve` |
//! | 装配者那一侧（`principal/bridge.rs` 的 `Roster`） | `Derive` ＋ `Bind` |
//! ⇒ **三条"问"的 ＋ 四条"定"的**，一面一条。而"问的一条与定的那条同一面"是有后果的，且量得
//! 出来：**持树者手里那枚门牌做得出 `Adopt`**（把一条号领到自己底下——`probe-rule` 量的那一格：
//! `adopt(q)` 之后 `Trunk(p)` 立刻答 `Denied`），而它一辈子只用两条**读**的。这与 operator 拆
//! 七格是同一条理由（`find` 会交出能力 ⇒ 不与只读那几条合成一位）。
//! # 面**就是那一枚门牌**（这一族与 operator 不同的一格）
//! operator 那一族有会话：面在**会话入口的记号**上（`ask()`），服务端从它自己表里那枚问话孔

use super::frame::Wire;

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
        Wire::Resolve(_) => Ask,
        Wire::Sire(_) => Ask,
        Wire::Heir(_, _) => Ask,
        Wire::Bind(_, _) => Set,
        Wire::Derive(_) => Set,
        Wire::Adopt(_) => Set,
        Wire::Waive => Set,
        Wire::Drop => Set,
    }
}
