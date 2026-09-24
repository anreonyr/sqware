//! principal 的**帧那一半** —— 帧与码（内核那两只手的别名在 [`call`](super::call)）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 答话那一格读不懂）一格都走不到。拆开之后这一份**只认
//! `env` 与同层 `core`**——**一处例外**：末尾那条"面不相撞"的编译期断言看得见
//! `crate::coalition`（常量对，不进机器）。宿主靶能把它逐字编进去
//! 跑判据；适配那半（`opened_by` 那种内核手的别名）留在 `call.rs`。
//!
//! 本文件**不做裁决**：名册与谱系的规矩全在 [`core`](super::core)。这里只有三件事——
//! 编一帧 / 解一帧、把失败域翻成答话码、把答案编进答话那一格。
//!
//! # 帧（一处上界各一格）
//!
//! ```text
//!   Ask    [0] op   [1..9] a   [9..17] b          ASK_LEN   = 17
//!   Reply  [0] status  [1] flag  [2..10] a        REPLY_LEN = 10
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`RESOLVE`/`DERIVE`/`SIRE` 只填 `a`，`HEIR` 两格都填）；
//! 答话定长，故两侧都不用攒缓冲、也不用问长度。
//!
//! # 答案为什么不进失败表
//!
//! [`RESOLVE`] 的"没绑"与 [`SIRE`] 的"它是根"都是**诚实的答案**，不是失败：它们走
//! `status == OK` + `flag == 0`；"这条号树外"才走 `status == UNKNOWN`。三件事三个落点
//! （`None` 与 `Unknown` 分得开），[`fail_codes!`] 那本双射表于是只装真正的失败——
//! `OK` 那一格照旧是"一个失败都不是"。
//!
//! **`flag` 那一格不能省**：`PrincipalId(0)` 是**根**，不是"没有"——`a` 那一格里的 0 是一个
//! 合法答案，故"有没有"只能另占一格。

use super::core::{Fail, PrincipalId};
use crate::id::Id;
use env::Mark;

// ── 码 ──────────────────────────────────────────────────────

/// 七条线上动作——**与核心那七条同名**（核心另有 `unbind` / `clan` 两条**不上线**，见正文
/// 那张表）：线上与模型是同一件事的两层，不该各起一套词。
pub const BIND: u8 = 1;
pub const RESOLVE: u8 = 2;
pub const DERIVE: u8 = 3;
pub const SIRE: u8 = 4;
pub const HEIR: u8 = 5;
/// 转换 · 领：`a` = 目标号（发送者由内核盖章，报文里没有"我是谁"那一格）。
pub const ADOPT: u8 = 6;
/// 转换 · 弃：两格都空——它只认"发送者是谁"。
pub const WAIVE: u8 = 7;

/// 答话那一格：失败域前三格 + "读不懂"。
///
/// [`BAD`] 在失败表外（同板/树的先例）：它不是"哪个协议说的事"，是**这一问读不懂**。
pub const OK: u8 = 0;
pub const DENIED: u8 = 1;
pub const UNKNOWN: u8 = 2;
pub const NO_ROOM: u8 = 3;
pub const BAD: u8 = 4;

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
///
/// **照实记（谁是这一格的读者）**：**宿主靶**（`protocol-case` 的 `roster` 靶验"动作码在第 0
/// 字节、短一字节也读得出"）。**本族的服务不按它分派**（`answer` 收的是 `unpack_ask` 解出来的
/// 三格），故生产路径没有用家——与 `board` / `operator` 那两份不同（那两家的服务先读动作码再
/// 分派，故有真用家）。**四家 frame 同形**，且靶要验的那条性质只有这一句问得出来
/// （`unpack_ask` 要全长），故留着这一格、把读者写明。
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

/// 编一答：`OK` + **有没有** + 一个号（`RESOLVE` 的"绑没绑"、`SIRE` 的"有没有父"）。
pub fn reply_present(present: bool, at: PrincipalId) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[1] = present as u8;
    out[2..10].copy_from_slice(&at.to_bytes());
    out
}

/// 编一答：`OK` + 新派生出来的那个号（`DERIVE`）。
pub fn reply_value(p: PrincipalId) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[2..10].copy_from_slice(&p.to_bytes());
    out
}

/// 编一答：`OK` + 是 / 不是（`HEIR`）。
pub fn reply_yes(yes: bool) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[1] = yes as u8;
    out
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// **本表只装写的那两条与"查无此节点"**：读的答案（没绑 / 它是根）走 `OK` + `flag`，
    /// 不进这张表（见文件头）。
    bijective Fail; OK;
    Fail::Denied => DENIED,
    Fail::Unknown => UNKNOWN,
    Fail::NoRoom => NO_ROOM,
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
///
/// 与 rtc 那一面的 `rtc-back` 同一个形状、不同的记号：两块门牌的回信孔若刻同一个记号，
/// 同一张表里就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("principal-back");

/// 树上那块窗格的名字（门牌的第一段）：`/sys`。
pub const DIR: &str = "sys";

/// 本服务在树上的名字（门牌的第二段）：`/sys/principal`。
pub const NAME: &str = "principal";

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台那条 `the_three_back_marks_of_the_three_doors_do_not_collide`：三条路的回信
// 孔记号两两不同（`principal-back` / `coalition-back` / `line-back`）。这里钉得着的是**与盟籍
// 那一对**（本文件看得见 `crate::coalition`）；**与线那两对钉在 `lib.rs`**——线那一枚住在
// `driver::line::call`，而帧这一半要能在宿主靶里**单独**编（那个靶的模块树里没有 `driver`）。
//
// **照实记（这一条曾经一直是空的）**：跨面那一对原先写作 `Mark::of("board-back")`，而**那个名字
// 从来没有存在过**——板那条路的答话走码头（`system/board/client.rs`：问话孔只写、答话从板路
// 读），它没有 `*-back` 记号。故换成真在的那一条（见 `lib.rs`）。

const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
const _: () = assert!(BACK.get() != crate::coalition::frame::BACK.get());
