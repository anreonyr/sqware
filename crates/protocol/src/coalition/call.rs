//! coalition 的转发层 —— **帧、码**，与答话的三种编法。
//!
//! 本文件**不做裁决**：盟册的规矩全在 [`core`](super::core)。这里只有三件事——
//! 编一帧 / 解一帧、把失败域翻成答话码、把答案编进答话那一格。
//!
//! # 帧（与 [`principal`](crate::principal::call) 同一形状）
//!
//! ```text
//!   Ask    [0] op   [1..9] a   [9..17] b          ASK_LEN   = 17
//!   Reply  [0] status  [1] flag  [2..10] a        REPLY_LEN = 10
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`FOUND` 两格都空，`ENTER` / `LEAVE` 只用 `a`，
//! `AMID` 两格都用）；答话定长，故两侧都不用攒缓冲、也不用问长度。
//!
//! **报文里没有"我是谁"这一格**：发送者由内核在 `Push` 那一刻盖章，Server 拿去名册问。
//!
//! # 编答的助手**少一个**
//!
//! [`principal`](crate::principal::call) 有 `reply_present`（"有没有一条号"），这里不需要——
//! 本族没有"可能没有的一条号"那种答案（`found` 必有号，`amid` 是是非）。**帮手少一个，
//! 是原语少一条的余数。**
//!
//! # 码的数字**不照抄 principal**
//!
//! 同一个概念 `UNKNOWN`，operator 那一面是 1、principal 那一面是 2——三家各按**自己失败域
//! 的顺序**排、`BAD` 收尾。故本族按自己的两格排（见 [`fail_codes!`] 那张表）：照抄别家只会
//! 让自己表里空出一个号。

use super::core::{CoalitionId, Fail};

// ── 码 ──────────────────────────────────────────────────────

/// 四条线上动作——**与核心那四条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
///
/// `band` / `bloc` 不在其中：它们**今天不上线**（答案是一串号，带回来要另开一种帧形——
/// 见正文）。
pub const FOUND: u8 = 1;
pub const ENTER: u8 = 2;
pub const LEAVE: u8 = 3;
pub const AMID: u8 = 4;

/// 答话那一格：失败域那两格 + "读不懂"。
///
/// [`BAD`] 在失败表外（同板 / 树 / 身份服务那三家的先例）：它不是"哪个协议说的事"，
/// 是**这一问读不懂**。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const NO_ROOM: u8 = 2;
pub const BAD: u8 = 3;

/// 一问的长度：动作码 + 两个 8 字节的号。
pub const ASK_LEN: usize = 1 + 8 + 8;

/// 一答的长度：状态 + 有没有 + 一个 8 字节的答案。
pub const REPLY_LEN: usize = 1 + 1 + 8;

// ── 编 / 解 ─────────────────────────────────────────────────

/// 编一问：`a` / `b` 两格按动作码填（这条调用的两个号都是 `usize`，线上统一 8 字节小端）。
pub fn pack_ask(op: u8, a: u64, b: u64) -> [u8; ASK_LEN] {
    let mut out = [0u8; ASK_LEN];
    out[0] = op;
    out[1..9].copy_from_slice(&a.to_le_bytes());
    out[9..17].copy_from_slice(&b.to_le_bytes());
    out
}

/// 只读第一格**动作码**（空帧 ⇒ `None`：Server 据此答 [`BAD`]，不猜、不崩）。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问：`(动作码, a, b)`。**长度不对就是读不懂**（返 `None`，由 Server 答 [`BAD`]）。
pub fn unpack_ask(bytes: &[u8]) -> Option<(u8, u64, u64)> {
    if bytes.len() != ASK_LEN {
        return None;
    }
    let op = *bytes.first()?;
    let mut a = [0u8; 8];
    a.copy_from_slice(bytes.get(1..9)?);
    let mut b = [0u8; 8];
    b.copy_from_slice(bytes.get(9..17)?);
    Some((op, u64::from_le_bytes(a), u64::from_le_bytes(b)))
}

/// 解一答：`(状态, 有没有, 答案)`。**长度不对就答 `None`**（读的人按"这一趟没走到"处理）。
pub fn unpack_reply(bytes: &[u8]) -> Option<(u8, u8, u64)> {
    if bytes.len() != REPLY_LEN {
        return None;
    }
    let status = *bytes.first()?;
    let flag = *bytes.get(1)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(bytes.get(2..10)?);
    Some((status, flag, u64::from_le_bytes(a)))
}

/// 编一答：只有状态那一格（失败，或读不懂）。
pub fn reply_status(code: u8) -> [u8; REPLY_LEN] {
    let mut out = [0u8; REPLY_LEN];
    out[0] = code;
    out
}

/// 编一答：`OK` + 一枚盟的号（`FOUND`）。
///
/// **`flag` 那一格不用**：`found` 必有号（零号也是合法答案），没有"没有"这一档。
pub fn reply_value(c: CoalitionId) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[2..10].copy_from_slice(&c.to_bytes());
    out
}

/// 编一答：`OK` + 是 / 不是（`AMID`）。
pub fn reply_yes(yes: bool) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[1] = yes as u8;
    out
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// 两格，**没有 `Denied`**：盟无主，没有一处"你得请谁来做"的判断。数字按本族失败域的
    /// 顺序排（`BAD` 收尾且在表外）——别家同一个概念排的是别的号，那不是约定。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::NoRoom => NO_ROOM,
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────

/// **这扇门是谁开的**（`Reserve` 的第二格）：客侧靠它认出"答话的是谁"——门牌那一枚是
/// 别人挂的，故只能读它；副本共享同一事实、转手不变。
pub use crate::session::call::opened_by;

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
///
/// 与另几面的 `*-back` 同一个形状、不同的记号：同一张表里两面的回信孔若刻同一个记号，
/// 就分不出这一枚是哪一面的。
pub const BACK: &str = "coalition-back";

/// 树上那块窗格的名字（门牌的第一段）：`/sys`。
pub const DIR: &str = "sys";

/// 本服务在树上的名字（门牌的第二段）：`/sys/coalition`。
pub const NAME: &str = "coalition";
