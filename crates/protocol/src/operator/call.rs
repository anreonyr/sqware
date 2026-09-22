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
//!                              [258 .. 266] 尾格（8 字节，含义由 op 定）
//!   Reply  [0] status                                  —— 一格的答（旧有）
//!          [0] status   [1] 条数   [2 ..] 号           —— 列
//!          [0] status   [1 ..] 名字                     —— 名（长度即名长）
//! ```
//!
//! **问话只有一种帧**：[`ASK_LEN`] 就是它的长度——段数封顶 [`Operator::PATH_MAX`]，
//! 故整帧定长、不预分配槽。名字按 `env::wire::NAME_LEN` 定长写（尾随 NUL 是填充）。
//! **答话有三种形状、各有各的上界**，服务端按 [`REPLY_MAX`] 备一只缓冲。
//!
//! **尾格是"随 op 变的那个号"**：`land` / `part` / `find` / `trim` 读它当**入口号**
//! （[`hang`] 换回来的那个号，不是"客人的 Pie 是几号"），`name` 读它当**条目的号**，
//! `list` 不看它。两个编号空间不同源，互相拿错正是旧树 `[33..41]` 那一格的病；
//! **答案那一侧则干脆没有这一格**：`find` 查到的那一枚经会话交进客人的表，报文里再放一个号
//! 只会多出一份两边都得认的约定。

use env::{Name, PieToken, TaskId};

use super::core::{EntryId, Fail, Operator, Unship, VestedBy};

use crate::session::{Claim, Seat};

// ── 码 ──────────────────────────────────────────────────────

/// 六个动作在报文里的码——**与核心那六条原语同名**（`land` / `part` / `find` / `trim` /
/// `list` / `name`）：线上与模型是同一件事的两层，不该各起一套词。
pub const LAND: u8 = 1;
pub const PART: u8 = 2;
pub const FIND: u8 = 3;
pub const TRIM: u8 = 4;
pub const LIST: u8 = 5;
pub const NAME: u8 = 6;

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

/// 整帧的长度：`op + 段数 + 8 段 + 尾格`。
pub const ASK_LEN: usize = 1 + 1 + Operator::PATH_MAX * env::wire::NAME_LEN + 8;

/// 段那一块在帧里的起点。
const SEGS: usize = 2;

/// 一帧「列」的答话：`[0] status [1] 条数 [2 ..] 号`（每条 8 字节）。
///
/// 一条 pane 本来就不超过 [`Operator::PANE_CAP`] 条 ⇒ **一趟答得完，没有"未完"那一格**
/// （对照 `coalition` 那一侧：盟籍没有上限，故那里必须带一格"未完"）。
pub const LIST_REPLY_LEN: usize = 2 + Operator::PANE_CAP * 8;

/// 一帧「名」的答话：`[0] status [1 ..] 名字`——**长度即名长**（变长就落在这一条上）。
///
/// 名字内容最多 `NAME_LEN - 1` 字节（要留终止 NUL，见 `env::wire::Name`）⇒ 整帧不超过
/// `NAME_LEN`。
///
/// **不用"一条空消息"说"没有这个号"**：核里 `msg.len() >= 1`（空不是消息），
/// 故那一格仍由状态字节说。
pub const NAME_REPLY_LEN: usize = 1 + (env::wire::NAME_LEN - 1);

/// 答话那一侧的上界：**服务端只备这一只缓冲**（三种答形里最大的那个）。
pub const REPLY_MAX: usize = LIST_REPLY_LEN;

const _: () = assert!(NAME_REPLY_LEN <= REPLY_MAX);

// ── 编 / 解 ─────────────────────────────────────────────────

/// 把一条路编成字节。尾格写 `tail`，它是什么意思由 `op` 定（见文件头）。
///
/// **段数写的是真实条数**（哪怕超过 [`Operator::PATH_MAX`]）：那样"路太长"由持树者按
/// [`Fail::Full`] 答出来，而不是在这里被悄悄截断成另一条路。
pub fn pack_ask(op: u8, path: &[Name], tail: [u8; 8]) -> [u8; ASK_LEN] {
    let mut out = [0u8; ASK_LEN];
    out[0] = op;
    out[1] = path.len().min(u8::MAX as usize) as u8;
    for (i, name) in path.iter().take(Operator::PATH_MAX).enumerate() {
        let at = SEGS + i * env::wire::NAME_LEN;
        out[at..at + env::wire::NAME_LEN].copy_from_slice(name.bytes());
    }
    out[SEGS + Operator::PATH_MAX * env::wire::NAME_LEN..].copy_from_slice(&tail);
    out
}

/// 只读第一格**动作码**（空帧 ⇒ `None`：持树者据此答 `BAD`，不猜、不崩）。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问：`(段数组, 真实段数, 尾格)`。**读不懂返 `None`**（持树者据此答 `BAD`）。
///
/// 长度为 [`ASK_LEN`] 是**帧的契约**（`pack_ask` 产出的就是这个长度），故短一字节即读不懂。
/// 尾格**原样**交出去：它是什么意思由 `op` 定（见文件头），本层不替它裁。
///
/// **段数为 0 与超过上限都在这里原样报出去**（不在这里裁决）：前者是一条空路（根），
/// 后者该由持树者答 `Full`——两条都由核心那六条原语的判据说了算。
pub fn unpack_ask(bytes: &[u8]) -> Option<([Name; Operator::PATH_MAX], usize, [u8; 8])> {
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
    let mut tail = [0u8; 8];
    tail.copy_from_slice(bytes.get(at..at + 8)?);
    Some((segs, count, tail))
}

/// 尾格**当入口号读**（`land` / `part` / `find` / `trim` 那一档）。
///
/// 全 0 就是 [`PieToken::NONE`]（"没带"）——那一格由持树者的 `land` 判读不懂，这里不裁。
pub fn entry_in(tail: [u8; 8]) -> PieToken {
    match PieToken::from_bytes(&tail) {
        Some(entry) => entry,
        None => PieToken::NONE,
    }
}

/// 尾格**当条目的号读**（`name` 那一档）。**不校验**：号还在不在由核心答
/// （[`Fail::Unknown`]）。
pub fn id_in(tail: [u8; 8]) -> EntryId {
    EntryId::from_bytes(tail)
}

// ── 答：一串号 / 一枚名字 ───────────────────────────────────

/// 一帧「列」的读数：号最多 [`Operator::PANE_CAP`] 枚。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    ids: [EntryId; Operator::PANE_CAP],
    n: usize,
}

impl Listing {
    /// 空的那一串。
    pub const fn new() -> Listing {
        Listing {
            ids: [EntryId::new(0); Operator::PANE_CAP],
            n: 0,
        }
    }

    /// 几枚。
    pub fn len(&self) -> usize {
        self.n
    }

    /// 一枚都没有。
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// 第 `at` 枚（越界 ⇒ `None`）。
    pub fn get(&self, at: usize) -> Option<EntryId> {
        if at < self.n {
            self.ids.get(at).copied()
        } else {
            None
        }
    }

    /// 按号序（就是帧里的次序）走一遍。
    pub fn iter(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.ids[..self.n].iter().copied()
    }

    /// 收一枚。**满了就丢**：一条 pane 本来就不超过 [`Operator::PANE_CAP`] 枚，
    /// 故这一格（`pack_list` 收够就停）不该被走到。
    fn push(&mut self, id: EntryId) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = id;
            self.n += 1;
        }
    }
}

/// 把一串号编成一帧答话（写进服务端那只缓冲），返**帧长**（= `2 + 8 × 枚数`）。
///
/// `ids` 收够 [`Operator::PANE_CAP`] 枚就停：一条 pane 本来就不超过它。
pub fn pack_list(out: &mut [u8; REPLY_MAX], ids: impl Iterator<Item = EntryId>) -> usize {
    let mut n = 0;
    for id in ids.take(Operator::PANE_CAP) {
        let at = 2 + n * 8;
        out[at..at + 8].copy_from_slice(&id.to_bytes());
        n += 1;
    }
    out[0] = OK;
    out[1] = n as u8;
    2 + n * 8
}

/// 解开一帧「列」：答话那一格不是 [`OK`] ⇒ `Err(那一格)`（读的人先看它，再看号）。
///
/// **帧长即条数**的对偶面：长度必须恰好 `2 + 8 × 条数`，短一字节即读不懂（[`BAD`]）。
pub fn read_list(bytes: &[u8]) -> Result<Listing, u8> {
    let Some((&code, rest)) = bytes.split_first() else {
        return Err(BAD);
    };
    if code != OK {
        return Err(code);
    }
    let Some((&count, body)) = rest.split_first() else {
        return Err(BAD);
    };
    let count = count as usize;
    if count > Operator::PANE_CAP || body.len() != count * 8 {
        return Err(BAD);
    }
    let mut listing = Listing::new();
    for chunk in body.chunks_exact(8) {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(chunk);
        listing.push(EntryId::from_bytes(raw));
    }
    Ok(listing)
}

/// 把一枚名字编成一帧答话（写进服务端那只缓冲），返**帧长**（= `1 + 名字内容长度`：
/// 长度即名长）。
pub fn pack_name(out: &mut [u8; REPLY_MAX], name: Name) -> usize {
    out[0] = OK;
    let text = name.text();
    out[1..1 + text.len()].copy_from_slice(text);
    1 + text.len()
}

/// 解开一帧「名」：答话那一格不是 [`OK`] ⇒ `Err(那一格)`。
///
/// 名字读不懂（空 / 太长 / 含 NUL / 不是 UTF-8）⇒ `Err(BAD)`：`Name` 那一侧的四格失败域
/// 在这里**归一格**——问的人能做的补救是同一件（这一帧坏了，重问）。
pub fn read_name(bytes: &[u8]) -> Result<Name, u8> {
    let Some((&code, text)) = bytes.split_first() else {
        return Err(BAD);
    };
    if code != OK {
        return Err(code);
    }
    Name::from_slice(text).map_err(|_| BAD)
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::NonEmpty => NONEMPTY,
    Fail::NotATile => NOTATILE,
    Fail::NotAPane => NOTAPANE,
    Fail::Full => FULL,
    Fail::Dead => DEAD,
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
