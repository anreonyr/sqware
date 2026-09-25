//! principal 的**帧那一半** —— 帧与码（内核那两只手的别名在 `protocol` 那一侧的 `call`）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 答话那一格读不懂）一格都走不到。拆开之后这一份**只认
//! `env` 与同层 `core`**——**一处例外**：末尾那条"面不相撞"的编译期断言看得见
//! `crate::system::coalition`（常量对，不进机器）。宿主靶能把它逐字编进去
//! 跑判据；适配那半（`opened_by` 那种内核手的别名）留在 `call.rs`。
//!
//! 本文件**不做裁决**：名册与谱系的规矩全在 [`core`](super::core)。这里只有三件事——
//! 把失败域翻成答话码、把答案编进答话那一格、以及**本族**那几格码与记号。
//!
//! # 帧（两族同形，故只有一份）
//!
//! ```text
//!   Ask    [0] op   [1..9] a   [9..17] b          ASK_LEN   = 17
//!   Reply  [0] status  [1] flag  [2..10] a        REPLY_LEN = 10
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`RESOLVE`/`DERIVE`/`SIRE` 只填 `a`，`HEIR` 两格都填）；
//! 答话定长，故两侧都不用攒缓冲、也不用问长度。**长度与编 / 解那几手的本体在
//! [`crate::frame`]**（coalition 那一份一字不差），本文件把它们按本族的名字转出来。
//!
//! # 答案为什么不进失败表
//!
//! [`RESOLVE`] 的"没绑"与 [`SIRE`] 的"它是根"都是**诚实的答案**，不是失败：它们走
//! `status == OK` + `flag == 0`；"这条号树外"才走 `status == UNKNOWN`。三件事三个落点
//! （`None` 与 `Unknown` 分得开），`fail_codes!` 那本双射表于是只装真正的失败——
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

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格：失败域那几格 + "读不懂"。
///
/// [`BAD`] 在失败表外（同板/树的先例）：它不是"哪个协议说的事"，是**这一问读不懂**。
pub const DENIED: u8 = 1;
pub const UNKNOWN: u8 = 2;
pub const FULL: u8 = 3;
pub const BAD: u8 = 4;

// ── 帧骨架（两族同形的那一份）───────────────────────────────
//
// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——principal 与 coalition 同形，故只有
// 一份；这里只按本族的名字转出来（`call.rs` 那句 `pub use super::frame::*;` 照旧，调用点一处
// 都不用改）。**本族自己的**是下面那些：码、`reply_present`、失败表、记号。

pub use crate::frame::{ASK_LEN, REPLY_LEN, op_of, pack_ask, reply_status, reply_value, reply_yes, unpack_ask, unpack_reply};

/// 编一答：`OK` + **有没有** + 一个号（`RESOLVE` 的"绑没绑"、`SIRE` 的"有没有父"）。
///
/// **只本族有这一手**：另几家没有"可能没有的一条号"那种答案 ⇒ 一位用家，不搬去共享那一份。
pub fn reply_present(present: bool, at: PrincipalId) -> [u8; REPLY_LEN] {
    let mut out = reply_status(OK);
    out[1] = present as u8;
    out[2..10].copy_from_slice(&at.to_bytes());
    out
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// **本表只装写的那两条与"查无此节点"**：读的答案（没绑 / 它是根）走 `OK` + `flag`，
    /// 不进这张表（见文件头）。
    bijective Fail; OK;
    Fail::Denied => DENIED,
    Fail::Unknown => UNKNOWN,
    Fail::Full => FULL,
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
// 那一对**（本文件看得见 `crate::system::coalition`）；**与线那两对钉在 `lib.rs`**——线那一枚住在
// `driver::line::frame`，而帧这一半要能在宿主靶里**单独**编（那个靶的模块树里没有 `driver`）。
//
// **照实记（这一条曾经一直是空的）**：跨面那一对原先写作 `Mark::of("board-back")`，而**那个名字
// 从来没有存在过**——板那条路的答话走码头（`system/board/client.rs`：问话孔只写、答话从板路
// 读），它没有 `*-back` 记号。故换成真在的那一条（见 `lib.rs`）。

const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
const _: () = assert!(BACK.get() != crate::system::coalition::frame::BACK.get());
