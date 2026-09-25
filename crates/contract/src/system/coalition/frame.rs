//! coalition 的**帧那一半** —— 帧与码（内核那一只手的别名在 `protocol` 那一侧的 `call`）。
//!
//! 本文件**不做裁决**：盟册的规矩全在 [`core`](super::core)。这里只有三件事——
//! 把失败域翻成答话码、把答案编进答话那一格、以及**本族**那几格码 / 记号 / **窗**那一档。
//!
//! **照实记（这一份为什么拆出来）**：见 `principal/frame.rs` 的同一条——帧形的边角机器走不到，
//! 拆开之后这一份只认 `env` 与同层 `core`，宿主靶能逐字编进去跑判据。
//!
//! # 帧（与 `system::principal::call` 同一形状；窗那一档多一种答形）
//!
//! ```text
//!   Query  [0] op   [1..9] a   [9..17] b   [17..25] back     25
//!   Reply  [0] status  [1] flag  [2..10] a                   10
//!          [0] status  [1] 未完  [2] 条数  [3..] 号    SEQ_REPLY_LEN = 131
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`FOUND` 两格都空，`ENTER` / `LEAVE` 只用 `a`，
//! `AMID` 两格都用，`BAND` / `BLOC` 的 `b` 是**游标**）——而"这一条有几格"由下面的
//! [`Req`] / [`Wire`] 按类型说。答话的两种形状**各有各的上界**，服务端按 [`REPLY_MAX`]
//! 备一只缓冲。**前两种形状的本体在 [`crate::frame`]**——两族同形，故只有一份；本文件把它们
//! 按本族的名字转出来，**窗那一档**留在这里（只此一族）。
//!
//! **照实记（这一行原写"一问 17 字节"）**：那是 `back` 那一格落地之前抄的，此后一问一直是
//! `1 + 8 + 8 + 8 = 25`（同 `principal/frame.rs` 那条，详见 [`crate::frame`]）。
//!
//! **游标是阈值，说在 `b` 那一格**：`b = 游标 + 1`，`0` = 没有游标（从头取）。加一是有理的
//! ——**零号是真格子**（`PrincipalId::ROOT` 是 0、`CoalitionId(0)` 是一枚普通的盟），
//! 拿 0 当"没有"会把那一位漏掉。取的是**号 > 阈值**的那些，故没有"过期游标"这回事。
//!
//! **报文里没有"我是谁"这一格**：发送者由内核在 `Push` 那一刻盖章，Server 拿去名册问。
//!
//! # 编答的助手**少一个**
//!
//! `system::principal::call` 有 `reply_present`（"有没有一条号"），这里不需要——
//! 本族没有"可能没有的一条号"那种答案（`found` 必有号，`amid` 是是非）。**帮手少一个，
//! 是原语少一条的余数。**
//!
//! # 码的数字**不照抄 principal**
//!
//! 同一个概念 `UNKNOWN`，operator 那一面是 1、principal 那一面是 2——三家各按**自己失败域
//! 的顺序**排、`BAD` 收尾。故本族按自己的两格排（见 `fail_codes!` 那张表）：照抄别家只会
//! 让自己表里空出一个号。

use super::core::{CoalitionId, Fail, WINDOW_CAP, Window};
use crate::id::Id;
use crate::system::principal::core::PrincipalId;
use env::{Mark, PieToken};

// ── 码 ──────────────────────────────────────────────────────

/// 六条线上动作——**与核心那六条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
pub const FOUND: u8 = 1;
pub const ENTER: u8 = 2;
pub const LEAVE: u8 = 3;
pub const AMID: u8 = 4;
pub const BAND: u8 = 5;
pub const BLOC: u8 = 6;

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格：失败域那两格 + "读不懂"。
///
/// [`BAD`] 在失败表外（同板 / 树 / 身份服务那三家的先例）：它不是"哪个协议说的事"，
/// 是**这一问读不懂**。
pub const UNKNOWN: u8 = 1;
pub const FULL: u8 = 2;
pub const BAD: u8 = 3;

// ── 帧骨架（两族同形的那一份）───────────────────────────────
//
// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——coalition 与 principal 同形（这一族
// 的帧就是照它立的），故只有一份；这里只按本族的名字转出来（`call.rs` 那句
// `pub use super::frame::*;` 照旧，调用点一处都不用改）。

pub use crate::frame::{Query, REPLY_LEN, reply_status, reply_value, reply_yes, unpack_reply};

// ── 一问：一条动作一格 ──────────────────────────────────────

/// **一问的形状**——一条动作一格（同 `principal/frame.rs` 那条照实记：它替掉了
/// `pack_ask(op, a, b, back)` 那种"任何一枚码配上任何两格数"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req {
    /// `FOUND`：铸一枚新盟——**两格都空**。
    Found,
    /// `ENTER`：进 `a` 那一枚盟。
    Enter(CoalitionId),
    /// `LEAVE`：离 `a` 那一枚盟。
    Leave(CoalitionId),
    /// `AMID`：`a` 此刻在 `b` 那一枚盟里吗（**两格都用**）。
    Amid(PrincipalId, CoalitionId),
    /// `BAND`：读 `a` 那一枚盟的盟籍；`b` = **游标**（从哪一枚之后接着读）。
    Band(CoalitionId, Option<PrincipalId>),
    /// `BLOC`：读 `a` 那一位在哪些盟里；`b` = **游标**。
    Bloc(PrincipalId, Option<CoalitionId>),
}

impl Req {
    /// 编成线上那一形；`back` = **这一趟的回信孔在对端表里的号**（运输那一格，不是荷载）。
    pub fn query(self, back: PieToken) -> Query {
        let (op, a, b) = match self {
            Req::Found => (FOUND, 0, 0),
            Req::Enter(c) => (ENTER, c.get() as u64, 0),
            Req::Leave(c) => (LEAVE, c.get() as u64, 0),
            Req::Amid(p, c) => (AMID, p.get() as u64, c.get() as u64),
            Req::Band(c, after) => (BAND, c.get() as u64, cursor_of(after)),
            Req::Bloc(p, after) => (BLOC, p.get() as u64, cursor_of(after)),
        };
        Query { op, a, b, back }
    }
}

/// **收进来的一问**（那两格号已经解成模型类型 / 游标）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Found,
    Enter(CoalitionId),
    Leave(CoalitionId),
    Amid(PrincipalId, CoalitionId),
    Band(CoalitionId, Option<PrincipalId>),
    Bloc(PrincipalId, Option<CoalitionId>),
}

impl Wire {
    /// 解一问：`(读出来的动作, 回信孔那一格)`——**动作读不出来给内层那个 `None`**（表外的动作码：
    /// 这一问**有回信的路**，只是这一码我不认 ⇒ 持册者答一句 `BAD`）；**长度不对给外层那个
    /// `None`**（连"往哪回"都没有 ⇒ 不动账、也不回话）。
    pub fn take(bytes: &[u8]) -> Option<(Option<Wire>, PieToken)> {
        if bytes.len() != Query::LEN {
            return None;
        }
        let q = Query::fetch(bytes)?;
        let ask = match q.op {
            FOUND => Some(Wire::Found),
            ENTER => Some(Wire::Enter(CoalitionId::new(q.a as usize))),
            LEAVE => Some(Wire::Leave(CoalitionId::new(q.a as usize))),
            AMID => Some(Wire::Amid(
                PrincipalId::new(q.a as usize),
                CoalitionId::new(q.b as usize),
            )),
            BAND => Some(Wire::Band(
                CoalitionId::new(q.a as usize),
                cursor_in(q.b).map(|raw| PrincipalId::new(raw)),
            )),
            BLOC => Some(Wire::Bloc(
                PrincipalId::new(q.a as usize),
                cursor_in(q.b).map(|raw| CoalitionId::new(raw)),
            )),
            // 表外的动作码：这一码不是我的（但"往哪回"读得出来）。
            _ => None,
        };
        Some((ask, q.back))
    }
}

/// 一窗答话的长度：状态 + **未完** + 条数 + [`WINDOW_CAP`] 枚号。
///
/// 盟籍没有上限（一格盟可以有很多人），故这一族**必须带"未完"那一格**——窗装不下是常态；
/// 对照 operator 那一侧：一条 pane 本来就不超过 `PANE_CAP`，故那边不用带。
pub const SEQ_REPLY_LEN: usize = 1 + 1 + 1 + WINDOW_CAP * 8;

/// 答话那一侧的上界：**服务端只备这一只缓冲**（两种答形里大的那个）。
pub const REPLY_MAX: usize = SEQ_REPLY_LEN;

const _: () = assert!(REPLY_LEN <= REPLY_MAX);

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
        out[at..at + 8].copy_from_slice(&id.to_bytes());
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
        T::from_bytes(raw)
    });
    Ok(Window::gather(more, ids))
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// 两格，**没有 `Denied`**：盟无主，没有一处"你得请谁来做"的判断。数字按本族失败域的
    /// 顺序排（`BAD` 收尾且在表外）——别家同一个概念排的是别的号，那不是约定。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Full => FULL,
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
// 钉在 `crate::system::principal::frame`，**与线那一对**钉在 `lib.rs`——线那一枚住在 `driver::line::frame`，
// 而这一份要能在宿主靶里**单独**编（那个靶的模块树里没有 `driver`）。
const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
