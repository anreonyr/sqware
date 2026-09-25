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
//!   Query  [0] op   [1..9] a   [9..17] b   [17..25] back     25
//!   Reply  [0] status  [1] flag  [2..10] a                   10
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`RESOLVE`/`DERIVE`/`SIRE` 只填 `a`，`HEIR` 两格都填）——
//! 而"这一条有几格"由下面的 [`Req`] / [`Wire`] **按类型说**（不再是一枚裸码当形参）。
//! 答话定长，故两侧都不用攒缓冲、也不用问长度。**形状本体在 [`crate::frame`]**
//! （coalition 那一份一字不差），本文件把它们按本族的名字转出来。
//!
//! **照实记（这两行原写"一问 17 字节"）**：那是 `back` 那一格落地之前抄的，此后一问一直是
//! `1 + 8 + 8 + 8 = 25`；这一刀把它改真（详见 [`crate::frame`] 里那条照实记）。
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
use env::{Mark, PieToken, TaskId};

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

pub use crate::frame::{Query, REPLY_LEN, reply_status, reply_value, reply_yes, unpack_reply};

// ── 一问：一条动作一格 ──────────────────────────────────────

/// **一问的形状**——一条动作一格：`a` / `b` 两格在该动作里有几个就有几个（"只填 a"那几条
/// **再没有第二个号可填**）。
///
/// **照实记（它替掉了什么）**：从前是 `pack_ask(op: u8, a: u64, b: u64, back)`——**任何一枚码都能
/// 配上任何两格数**，"这一条有几格"只活在调用方与服务端那两段 `match` 里；错配**编得过**。
/// 现在形状由类型说，编解码由 [`Query`] 那张表生成。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req {
    /// `BIND`：`a` = 哪一枚线程、`b` = 绑成谁。
    Bind(TaskId, PrincipalId),
    /// `RESOLVE`：这一枚线程此刻代表谁（`a` 一格）。
    Resolve(TaskId),
    /// `DERIVE`：从 `a` 派生一条新号。
    Derive(PrincipalId),
    /// `ADOPT`：转换 · 领——认 `a` 为父。
    Adopt(PrincipalId),
    /// `WAIVE`：转换 · 弃——**两格都空**（它只认"发送者是谁"）。
    Waive,
    /// `SIRE`：`a` 的父是谁。
    Sire(PrincipalId),
    /// `HEIR`：`a` 在 `b` 那一支里吗（**两格都用**）。
    Heir(PrincipalId, PrincipalId),
}

impl Req {
    /// 编成线上那一形；`back` = **这一趟的回信孔在对端表里的号**（运输那一格，不是荷载）。
    pub fn query(self, back: PieToken) -> Query {
        let (op, a, b) = match self {
            Req::Bind(tid, p) => (BIND, tid.get() as u64, p.get() as u64),
            Req::Resolve(tid) => (RESOLVE, tid.get() as u64, 0),
            Req::Derive(p) => (DERIVE, p.get() as u64, 0),
            Req::Adopt(p) => (ADOPT, p.get() as u64, 0),
            Req::Waive => (WAIVE, 0, 0),
            Req::Sire(p) => (SIRE, p.get() as u64, 0),
            Req::Heir(a2, b2) => (HEIR, a2.get() as u64, b2.get() as u64),
        };
        Query { op, a, b, back }
    }
}

/// **收进来的一问**（那两格号已经解成两个模型类型——线上只有数字，意义在动作码那一格）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Bind(TaskId, PrincipalId),
    Resolve(TaskId),
    Derive(PrincipalId),
    Adopt(PrincipalId),
    Waive,
    Sire(PrincipalId),
    Heir(PrincipalId, PrincipalId),
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
            BIND => Some(Wire::Bind(
                TaskId::new(q.a as usize),
                PrincipalId::new(q.b as usize),
            )),
            RESOLVE => Some(Wire::Resolve(TaskId::new(q.a as usize))),
            DERIVE => Some(Wire::Derive(PrincipalId::new(q.a as usize))),
            ADOPT => Some(Wire::Adopt(PrincipalId::new(q.a as usize))),
            WAIVE => Some(Wire::Waive),
            SIRE => Some(Wire::Sire(PrincipalId::new(q.a as usize))),
            HEIR => Some(Wire::Heir(
                PrincipalId::new(q.a as usize),
                PrincipalId::new(q.b as usize),
            )),
            // 表外的动作码：这一码不是我的（但"往哪回"读得出来）。
            _ => None,
        };
        Some((ask, q.back))
    }
}

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
