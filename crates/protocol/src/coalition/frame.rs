//! coalition 的**帧那一半** —— 帧与码（内核那一只手的别名在 [`call`](super::call)）。
//!
//! 本文件**不做裁决**：盟册的规矩全在 [`core`](super::core)。这里只有三件事——
//! 编一帧 / 解一帧、把失败域翻成答话码、把答案编进答话那一格。
//!
//! **照实记（这一份为什么拆出来）**：见 `principal/frame.rs` 的同一条——帧形的边角机器走不到，
//! 拆开之后这一份只认 `env` 与同层 `core`，宿主靶能逐字编进去跑判据。
//!
//! 本文件**不做裁决**：盟册的规矩全在 [`core`](super::core)。这里只有三件事——
//! 编一帧 / 解一帧、把失败域翻成答话码、把答案编进答话那一格。
//!
//! # 帧（与 [`principal`](crate::principal::call) 同一形状；窗那一档多一种答形）
//!
//! ```text
//!   Ask    [0] op   [1..9] a   [9..17] b          ASK_LEN   = 17
//!   Reply  [0] status  [1] flag  [2..10] a        REPLY_LEN = 10
//!          [0] status  [1] 未完  [2] 条数  [3..] 号    SEQ_REPLY_LEN = 131
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`FOUND` 两格都空，`ENTER` / `LEAVE` 只用 `a`，
//! `AMID` 两格都用）；答话的两种形状**各有各的上界**，服务端按 [`REPLY_MAX`] 备一只缓冲。
//!
//! **游标是阈值，说在 `b` 那一格**：`b = 游标 + 1`，`0` = 没有游标（从头取）。加一是有理的
//! ——**零号是真格子**（`PrincipalId::ROOT` 是 0、`CoalitionId(0)` 是一枚普通的盟），
//! 拿 0 当"没有"会把那一位漏掉。取的是**号 > 阈值**的那些，故没有"过期游标"这回事。
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

use super::core::{CoalitionId, Fail, Id, WINDOW_CAP, Window};
use env::Mark;

// ── 码 ──────────────────────────────────────────────────────

/// 六条线上动作——**与核心那六条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
pub const FOUND: u8 = 1;
pub const ENTER: u8 = 2;
pub const LEAVE: u8 = 3;
pub const AMID: u8 = 4;
pub const BAND: u8 = 5;
pub const BLOC: u8 = 6;

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

/// 一窗答话的长度：状态 + **未完** + 条数 + [`WINDOW_CAP`] 枚号。
///
/// 盟籍没有上限（一格盟可以有很多人），故这一族**必须带"未完"那一格**——窗装不下是常态；
/// 对照 operator 那一侧：一条 pane 本来就不超过 `PANE_CAP`，故那边不用带。
pub const SEQ_REPLY_LEN: usize = 1 + 1 + 1 + WINDOW_CAP * 8;

/// 答话那一侧的上界：**服务端只备这一只缓冲**（两种答形里大的那个）。
pub const REPLY_MAX: usize = SEQ_REPLY_LEN;

const _: () = assert!(REPLY_LEN <= REPLY_MAX);

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

// ── 窗：游标与一窗号 ────────────────────────────────────────

/// 游标那一格：**`b` = 游标 + 1**，`0` = 没有游标（从头取）。
///
/// 加一是那个双射：零号是真格子（`PrincipalId::ROOT` 是 0），拿 0 当"没有"会把它漏掉。
pub fn cursor_of<T: Id>(after: Option<T>) -> u64 {
    match after {
        Some(at) => at.get() as u64 + 1,
        None => 0,
    }
}

/// 游标那一格解回来（`0` ⇒ `None`；其余 ⇒ 裸号）。
pub fn cursor_in(b: u64) -> Option<usize> {
    if b == 0 { None } else { Some(b as usize - 1) }
}

/// 把一窗号编成一帧答话（写进服务端那只缓冲），返**帧长**（= `3 + 8 × 枚数`）。
pub fn pack_seq<T: Id>(out: &mut [u8; REPLY_MAX], window: &Window<T>) -> usize {
    out[0] = OK;
    out[1] = window.more() as u8;
    out[2] = window.len() as u8;
    for (i, id) in window.iter().enumerate() {
        let at = 3 + i * 8;
        out[at..at + 8].copy_from_slice(&(id.get() as u64).to_le_bytes());
    }
    3 + window.len() * 8
}

/// 解开一帧「窗答」：答话那一格不是 [`OK`] ⇒ `Err(那一格)`。
///
/// 帧长必须恰好 `3 + 8 × 条数`、条数不超过 [`WINDOW_CAP`]、未完那一格只许 0 / 1——
/// 短一字节即是读不懂（[`BAD`]）：这一族**不猜**。
pub fn read_seq<T: Id>(bytes: &[u8]) -> Result<Window<T>, u8> {
    let Some((&code, rest)) = bytes.split_first() else {
        return Err(BAD);
    };
    if code != OK {
        return Err(code);
    }
    let Some((&more, rest)) = rest.split_first() else {
        return Err(BAD);
    };
    let Some((&count, body)) = rest.split_first() else {
        return Err(BAD);
    };
    let more = match more {
        0 => false,
        1 => true,
        _ => return Err(BAD),
    };
    let count = count as usize;
    if count > WINDOW_CAP || body.len() != count * 8 {
        return Err(BAD);
    }
    let ids = body.chunks_exact(8).map(|chunk| {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(chunk);
        T::new(u64::from_le_bytes(raw) as usize)
    });
    Ok(Window::gather(more, ids))
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

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
///
/// 与另几面的 `*-back` 同一个形状、不同的记号：同一张表里两面的回信孔若刻同一个记号，
/// 就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("coalition-back");

/// 树上那块窗格的名字（门牌的第一段）：`/sys`。
pub const DIR: &str = "sys";

/// 本服务在树上的名字（门牌的第二段）：`/sys/coalition`。
pub const NAME: &str = "coalition";

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台那条 `the_three_back_marks_of_the_three_doors_do_not_collide`；**与名册那一对**
// 钉在 `crate::principal::frame`，**与线那一对**钉在 `lib.rs`——线那一枚住在 `driver::line::call`，
// 而这一份要能在宿主靶里**单独**编（那个靶的模块树里没有 `driver`）。
const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
