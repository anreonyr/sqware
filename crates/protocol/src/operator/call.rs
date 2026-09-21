//! operator 的转发层 —— **帧、码，与内核那几只手的别名**。
//!
//! 本文件**不做裁决**：树上的规矩（谁能落、什么时候剔死）全在 [`core`](super::core)。
//! 这里只有三件事——**编一帧 / 解一帧**、把"不在我表里"翻成 `None`、把失败域翻成答话码。
//!
//! 判据只有一条可机械检查的纪律——
//!
//! > 本文件里的 `if` / `match` 一处裁决也没有，只有两张对照表（失败域 ↔ 答话码、
//! > 会话失败域 ↔ 板失败域）与定长编解码。
//!
//! # 帧
//!
//! ```text
//!   Ask    [0] op   [1] 段数   [2 .. 258] 段（8 段定长，每段 32 字节，尾随 NUL 是填充）
//!                              [258 .. 266] 入口号（只有 `land` 用）
//!   Reply  [0] status
//! ```
//!
//! **一处上界**：本协议只有一种帧，[`ASK_LEN`] 就是它的长度——段数封顶 [`Operator::PATH_MAX`]，
//! 故整帧定长、不预分配槽。名字按 `env::wire::NAME_LEN` 定长写（尾随 NUL 是填充）。
//!
//! **入口号是"交出去之后换回来的那个号"**：客人把 Pie 交给持树者（[`hang`]），拿回"种在
//! 持树者表里"的那个号，再写进帧里——不是"客人的 Pie 是几号"。两个编号空间不同源，互相
//! 拿错正是旧树 `[33..41]` 那一格的病；**答案那一侧则干脆没有这一格**：`find` 查到的那一枚
//! 经会话交进客人的表，报文里再放一个号只会多出一份两边都得认的约定。

use env::{Name, PieToken, TaskId};

use super::core::{Fail, Operator, Unship, VestedBy};

use crate::session::{Claim, Seat};

// ── 码 ──────────────────────────────────────────────────────

/// 四个动作在报文里的码——**与核心那四条原语同名**（`land` / `part` / `find` / `trim`）：
/// 线上与模型是同一件事的两层，不该各起一套词。
///
/// `list` 不在其中：**今天不上线**（它的答案是一串名字，带回来要另开帧形——见正文）。
pub const LAND: u8 = 1;
pub const PART: u8 = 2;
pub const FIND: u8 = 3;
pub const TRIM: u8 = 4;

/// 答话那一格。**前六格与 [`Fail`] 一一对应**（`OK` = 一个失败都不是），第七格不是失败域
/// 的：这一问读不懂（帧坏了 ⇒ 不猜、不崩）。
///
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持树者那一侧编、客人那一侧读）。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const NONEMPTY: u8 = 2;
pub const NOTATILE: u8 = 3;
pub const NOTAPANE: u8 = 4;
pub const FULL: u8 = 5;
pub const DEAD: u8 = 6;
pub const BAD: u8 = 7;

/// 整帧的长度：`op + 段数 + 8 段 + 入口号`。
pub const ASK_LEN: usize = 1 + 1 + Operator::PATH_MAX * env::wire::NAME_LEN + 8;

/// 段那一块在帧里的起点。
const SEGS: usize = 2;

// ── 编 / 解 ─────────────────────────────────────────────────

/// 把一条路编成字节。`seed` 只有 [`LAND`] 用得上。
///
/// **段数写的是真实条数**（哪怕超过 [`Operator::PATH_MAX`]）：那样"路太长"由持树者按
/// [`Fail::Full`] 答出来，而不是在这里被悄悄截断成另一条路。
pub fn pack_ask(op: u8, path: &[Name], seed: Option<PieToken>) -> [u8; ASK_LEN] {
    let mut out = [0u8; ASK_LEN];
    out[0] = op;
    out[1] = path.len().min(u8::MAX as usize) as u8;
    for (i, name) in path.iter().take(Operator::PATH_MAX).enumerate() {
        let at = SEGS + i * env::wire::NAME_LEN;
        out[at..at + env::wire::NAME_LEN].copy_from_slice(name.bytes());
    }
    if let Some(seed) = seed {
        out[SEGS + Operator::PATH_MAX * env::wire::NAME_LEN..].copy_from_slice(&seed.to_bytes());
    }
    out
}

/// 只读第一格**动作码**（空帧 ⇒ `None`：持树者据此答 `BAD`，不猜、不崩）。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问：`(段数组, 真实段数, 入口号)`。**读不懂返 `None`**（持树者据此答 `BAD`）。
///
/// 长度为 [`ASK_LEN`] 是**帧的契约**（`pack_ask` 产出的就是这个长度），故短一字节即读不懂。
/// 入口号按 [`PieToken::NONE`] = "没带"解——令牌自 1 起，0 是内核的越界哨兵。
///
/// **段数为 0 与超过上限都在这里原样报出去**（不在这里裁决）：前者是一条空路（根），
/// 后者该由持树者答 `Full`——两条都由核心那四条原语的判据说了算。
pub fn unpack_ask(bytes: &[u8]) -> Option<([Name; Operator::PATH_MAX], usize, PieToken)> {
    let count = *bytes.get(1)? as usize;
    let mut segs = [Name::EMPTY; Operator::PATH_MAX];
    // **只解前 `count` 段**：`pack_ask` 只填了那么多，剩下的段位是零填充——空段不是名字，
    // 拿它去解会把一整帧判成"读不懂"（真机实测：四格全答 `BAD` 就栽在这里）。
    let filled = count.min(Operator::PATH_MAX);
    for (i, slot) in segs.iter_mut().enumerate().take(filled) {
        let at = SEGS + i * env::wire::NAME_LEN;
        let raw = bytes.get(at..at + env::wire::NAME_LEN)?;
        let mut buf = [0u8; env::wire::NAME_LEN];
        buf.copy_from_slice(raw);
        *slot = Name::from_bytes(buf).ok()?;
    }
    let at = SEGS + Operator::PATH_MAX * env::wire::NAME_LEN;
    let seed = PieToken::from_bytes(bytes.get(at..at + 8)?)?;
    Some((segs, count, seed))
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

/// 失败域 → 答话那一格（`None` = 一个失败都不是）。
pub const fn fail_to_code(fail: Option<Fail>) -> u8 {
    match fail {
        None => OK,
        Some(Fail::Unknown) => UNKNOWN,
        Some(Fail::NonEmpty) => NONEMPTY,
        Some(Fail::NotATile) => NOTATILE,
        Some(Fail::NotAPane) => NOTAPANE,
        Some(Fail::Full) => FULL,
        Some(Fail::Dead) => DEAD,
    }
}

/// 答话那一格 → 失败域。`OK`（没失败）与 `BAD`（这一问读不懂）**都不是失败域里的东西**，
/// 故两者同一格答 `None`——读的人靠 [`op_of`] / [`unpack_ask`] 先分流。
///
/// **本表是双射**（六个失败一格一码），故反向答得回来。
pub const fn code_to_fail(code: u8) -> Option<Fail> {
    match code {
        UNKNOWN => Some(Fail::Unknown),
        NONEMPTY => Some(Fail::NonEmpty),
        NOTATILE => Some(Fail::NotATile),
        NOTAPANE => Some(Fail::NotAPane),
        FULL => Some(Fail::Full),
        DEAD => Some(Fail::Dead),
        _ => None,
    }
}

/// 会话的失败域 → 树的失败域：**"它不在"是一条判据**，故两边只留一个名字
/// （[`Fail::Unknown`]）。
///
/// 「一笔都没到」与「到了一些、不齐」在上面那一层都归 `Unknown` / `Full`：树这一侧只有
/// 一格答话码，问的人按它决定要不要重问。
pub fn map_claim(claim: Claim) -> Fail {
    match claim {
        // 我的表读不动 ⇒ 这一问没有答案（与"它不在"同一格：都不是"树上答了没有"）。
        Claim::Unread => Fail::Unknown,
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

/// 装一条路的失败域 → 树的失败域。
///
/// 名字 / 资源上的毛病（名字非法、同名已装、铸不出孔）是**调用方写错了** ⇒ `Unknown`
/// （树上没有这一格可指）；交不出去（对端已不在）⇒ `Unknown`（"它不在"）；账腾不出来 ⇒ `Full`。
pub fn map_seat(seat: Seat) -> Fail {
    match seat {
        Seat::NoName => Fail::Unknown,
        Seat::NoHole => Fail::Unknown,
        Seat::NoSeed => Fail::Unknown,
        Seat::NoRoom => Fail::Full,
    }
}

// ── 内核那几只手的别名 ──────────────────────────────────────

// ── 一个调用的三个事实：身体在 `session::call`，这里只取名字 ──────────
//
// 三格是**一组**，三个名字读成同一句式的被动式事实、故等长（9/9/9）：
// **这枚是谁授的 / 这扇门是谁开的 / 这枚被标成什么**。树这一侧原先各抄一份
// （`probe`5 / `opened_by`9 / `mark_of`7），那一份已删（三格上三处的读法见 `vested_by`）。
pub use crate::session::call::{marked_as, opened_by, vested_by};

/// **卸下**：自释一份。剪掉或换掉一枚 `Tile` 时由核心叫它。
pub use crate::session::call::unship;

/// 立一棵树：把两个机制函数交给核心（核心因此不 `use` 内核）。
///
/// `const` 是为了它能当 `static` 的初值——树只有一棵，住在本域（`bin/operator`）。
pub const fn tree() -> Operator {
    let vested_by: VestedBy = vested_by;
    let unship: Unship = unship;
    Operator::new(vested_by, unship)
}

/// **交出去**：把调用方手里那一枚交给持树者（`Accord` 一份副本），返"种在持树者表里"的号。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：持树者查到名字时要**再授出**（`find` 的下场）——
/// 内核那道"持 `VEST` 才交得出去"的闸挡的就是"查到了却授不出去"。
/// 身体在 [`crate::session::call::ship`]。
pub use crate::session::call::ship as hang;

/// **授出**：把树上那一枚转授给调用方（`find` 的下场）。
///
/// 与 [`hang`] 同一份子集（`R|W|VEST`）：**拿到它的人可以再传**——那正是"一个名字指向
/// 一枚 Pie"的用法，故这里不替调用方裁剪。
/// **失败域是本模块的**（`Unknown`）：身体共用，失败值各自说。
pub fn give(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    crate::session::call::ship(entry, to).map_err(|()| Fail::Unknown)
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────
//
// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）。
pub const ASK_MARK: &str = "ask";

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）。
pub const TIP_MARK: &str = "tip";

/// 提示之路的名字（两侧共用：持树者那侧不用它——它那一枚是自己铸的；引导域用它把
/// 认来的那一枚挂在"名字 → 我手里的一枚"这张账上，好让编排域按名来要）。
pub const TIP_NAME: &str = "operator-tip";
