//! **定长一问一答**那一族帧的骨架 —— `principal` 与 `coalition` **同形的那一份**。
//!
//! ```text
//!   Ask    [0] op  [1..9] a  [9..17] b  [17..25] back    ASK_LEN   = 25
//!   Reply  [0] status  [1] flag  [2..10] a               REPLY_LEN = 10
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`RESOLVE`/`DERIVE`/`SIRE` 只填 `a`，`HEIR` 两格都填）；
//! 答话定长，故两侧都不用攒缓冲、也不用问长度。
//!
//! **`back` 那一格是"往哪回"**（末尾 8 字节）：客侧每趟铸一枚回信孔借给对端，
//! [`pack_ask`] 收的那一格就是**它在对端表里的号**——对端据此一次 `Reserve` 验出来，
//! 不必再扫自己的表按"谁给的 ＋ 记号"去找。这一格的名字两侧各按自己的角色念：
//! 客侧叫 **seed**（"种在你表里的那一枚"）、服务端叫 `back`，**[`pack_ask`] 的形参按服务端那一侧叫**
//! （读它的是服务端）。照实记那一刀的账见 [`pack_ask`] 与 `session::call::lend_out`。
//!
//! **这里只放"同形"的那几手**：两族的**码**（动作码与答话码）各按自己的失败域排、记号各是各的
//! （`principal-back` / `coalition-back`）、`fail_codes!` 表各装各的失败域——那些都留在各族自己的
//! `frame.rs`。**身体搬到这里，两处只留各自的名字**（同 `session::call` 那条纪律）。
//!
//! **两样不在这里，各有各的理由**：`reply_present`（"有没有一条号"）只有 principal 用 ⇒ **一位
//! 用家不搬**；`cursor_of` / `pack_seq` / `read_seq`（**窗**那一档）只有 coalition 用 ⇒ 留在那边
//! ——operator 的"一条 pane 本来就有顶"不需要 `more` 那一格，故窗不是这一族的共性。
//!
//! 本文件是**协议层**的东西，与 `id.rs` / `fail_codes.rs` 同一种编法：宿主靶**真依赖**它。

use crate::fail_codes::OK;
use crate::id::Id;
use env::PieToken;

// ── 帧长 ────────────────────────────────────────────────────

/// 一问的长度：动作码 + 两个 8 字节的号 + **回信孔那一格**（[`PieToken::WIDTH`]）。
///
/// 那一格不占 8 字节的常数、而按 `PieToken::WIDTH` 取：号的宽度是**号自己的事实**
/// （同 `Mark::WIDTH` 那条纪律），帧长不该替它说话。
pub const ASK_LEN: usize = 1 + 8 + 8 + PieToken::WIDTH;

/// 一答的长度：状态 + 有没有 + 一个 8 字节的答案。
pub const REPLY_LEN: usize = 1 + 1 + 8;

// ── 编 / 解 ─────────────────────────────────────────────────

/// 编一问：`a` / `b` 两格按动作码填（这条调用的两个号都是 `usize`，线上统一 8 字节小端）；
/// `back` = **回信孔在对端表里的号**（客侧 `lend_out` 的第二格）。
///
/// **照实记（`back` 那一格为什么在帧里）**：从前它不在——客侧 `lend` 把 `port::ship` 的第二格
/// （`to.seed()`，就是"我给你的那一枚在你表里是几号"）**扔了**，于是服务端只能**扫自己的表**
/// 按"谁给的 ＋ 记号"把那一枚认回来（`session::call::find`）。那一扫是每趟请求一遍全表，
/// 而 `Collect` 每枚还要算一次 `vestor`（吃全世界快照）——读数见
/// `programs/src/driver/rtc/main.rs` 与提交 `444d1f3` / `b58fda4`。
/// 把它放进帧里之后，服务端**一次 `Reserve` 就验完**（判据一字未改：谁开的 ＋ 记号）。
pub fn pack_ask(op: u8, a: u64, b: u64, back: PieToken) -> [u8; ASK_LEN] {
    let mut out = [0u8; ASK_LEN];
    out[0] = op;
    out[1..9].copy_from_slice(&a.to_le_bytes());
    out[9..17].copy_from_slice(&b.to_le_bytes());
    out[17..].copy_from_slice(&back.to_bytes());
    out
}

/// 只读第一格**动作码**（空帧 ⇒ `None`：Server 据此答 `BAD`，不猜、不崩）。
///
/// **照实记（谁是这一格的读者）**：**宿主靶**（`protocol-case` 的 `roster` 靶验"动作码在第 0
/// 字节、短一字节也读得出"）。**两族的服务都不按它分派**（`answer` 收的是 [`unpack_ask`] 解出来的
/// 四格），故生产路径没有用家；靶要验的那条性质只有这一句问得出来（[`unpack_ask`] 要全长），
/// 故留着这一格、把读者写明。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问：`(动作码, a, b, 回信孔那一格)`。**长度不对就是读不懂**（返 `None`，由 Server 答 `BAD`）。
///
/// `back` 是**在对端（读的这一侧）表里**的号，故服务端拿到它就能直接 `Reserve`——
/// 不必按发送者去扫表（见 [`pack_ask`] 的照实记）。
pub fn unpack_ask(bytes: &[u8]) -> Option<(u8, u64, u64, PieToken)> {
    if bytes.len() != ASK_LEN {
        return None;
    }
    let op = *bytes.first()?;
    let mut a = [0u8; 8];
    a.copy_from_slice(bytes.get(1..9)?);
    let mut b = [0u8; 8];
    b.copy_from_slice(bytes.get(9..17)?);
    let back = PieToken::from_bytes(bytes.get(17..ASK_LEN)?)?;
    Some((op, u64::from_le_bytes(a), u64::from_le_bytes(b), back))
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

/// 编一答：`OK` + 是 / 不是（principal 的 `HEIR`、coalition 的 `AMID`）。
pub fn reply_yes(yes: bool) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[1] = yes as u8;
    out
}

/// 编一答：`OK` + 一枚号（principal 是"新派生出来的那一条"，coalition 是"新铸的那一枚盟"）。
///
/// **号的类型是泛型**（[`Id`]）：两族的号是两种类型，而"填进 `[2..10]`"这件事一模一样。
///
/// **`flag` 那一格不用**：这一路的答案**必有**号（零号也是合法答案）——"有没有"是另一条路
/// （`reply_present`，只 principal 有）。
pub fn reply_value<T: Id>(at: T) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[2..10].copy_from_slice(&at.to_bytes());
    out
}
