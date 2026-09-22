//! operator 的转发层 —— **帧、码，与内核那几只手的别名**。
//!
//! 本文件**不做裁决**：树上的规矩（谁能落、什么时候剔死）全在 [`core`](super::core)。
//! 这里只有三件事——**编一帧 / 解一帧**、把"不在我表里"翻成 `None`、把失败域翻成答话码。
//!
//! 判据只有一条可机械检查的纪律——
//!
//! > 本文件里的 `if` / `match` 一处裁决也没有，只有两张对照表（失败域 ↔ 答话码、
//! > 会话失败域 ↔ 板失败域）与编解码。
//!
//! # 帧
//!
//! ```text
//!   Ask    seek   [0] op  [1] 段数  [2 .. 2+32k] 路                （k ≤ ROAD_MAX）
//!          list   [0] op  [1] 记    [2 .. 10]     号              （记：0 = 根 / 1 = 号）
//!          part   [0] op  [1] 记    [2 .. 10]     号  [10 .. 42] 名
//!          land   [0] op  [1] 记    [2 .. 10]     号  [10 .. 42] 名  [42 .. 50] 尾格
//!          find   [0] op  [1 .. 9] 号
//!          trim   同 find
//!          name   同 find
//!   Reply  [0] status                                    —— 一格的答
//!          [0] status   [1] 条数   [2 ..] 号             —— 列
//!          [0] status   [1 ..] 名字                       —— 名（长度即名长）
//!          [0] status   [1 .. 9] 号                       —— 号（`land` / `part` / `seek`，定长 9）
//! ```
//!
//! **问话一个动作一条形状**（不再是"一帧定长、尾格含义由 op 定"）：荷载收什么，帧里就写什么
//! ——没有一个"报法"字段可以填错，也没有第二个意思可读。最长的仍是 `seek` 那一条
//! （[`ASK_MAX`]，路封顶 [`Operator::ROAD_MAX`] 段），其余都落在十到五十字节。
//!
//! **尾格只剩 `land` 用**：入口那一枚经会话交出去（[`ship`] 换回来的那个号，不是"客人的 Pie
//! 是几号"），报文里走的只是"种在持树者表里的号"。两个编号空间不同源，互相拿错正是旧树
//! `[33..41]` 那一格的病。
//!
//! **答话有四种形状、各有各的上界**，服务端按 [`REPLY_MAX`] 备一只缓冲。

use env::Mark;
use env::{Name, PieToken, TaskId};

use super::core::{EntryId, Fail, Operator, Unship, VestedBy, Where};
use crate::session::{Claim, Seat};

// ── 码 ──────────────────────────────────────────────────────

/// 七个动作在报文里的码——**与核心那七条原语同名**（`land` / `part` / `find` / `trim` /
/// `list` / `seek` / `name`）：线上与模型是同一件事的两层，不该各起一套词。
pub const LAND: u8 = 1;
pub const PART: u8 = 2;
pub const FIND: u8 = 3;
pub const TRIM: u8 = 4;
pub const LIST: u8 = 5;
pub const NAME: u8 = 6;

/// **第七个动作**：把一条路**译成号**——名字只能走到这一格，往下一律按号。
///
/// 数字取 7 是白捡的：答话那一列里 `BAD` 也是 7，但**动作码与答话码本来就是两张表**
/// （今天 `LAND`..`NAME` 的 1..6 与 `UNKNOWN`..`DEAD` 的 1..6 已经重号），故两边各按各的序列。
pub const SEEK: u8 = 7;

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

/// 问话那一侧的上界：**最长那一条**（`seek`：`op + 段数 + 8 段名字`）。
///
/// 服务端按它备一只缓冲（收下来的帧不会超过它），各条问话的**实际**长度由 [`pack_ask`] 说话。
pub const ASK_MAX: usize = 2 + Operator::ROAD_MAX * env::wire::NAME_LEN;

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

/// 一帧「号」的答话：`[0] status [1 .. 9] 号`——**定长 9**（`land` / `part` / `seek` 答的那一格）。
///
/// 与「名」那一帧同一个道理：号是**数据**，故成败都写在这一帧里；但号不是变长的，
/// 故长度是死的 9——多一字节、少一字节都是读不懂（[`BAD`]）。
pub const ID_REPLY_LEN: usize = 1 + 8;

/// 答话那一侧的上界：**服务端只备这一只缓冲**（四种答形里最大的那个）。
pub const REPLY_MAX: usize = LIST_REPLY_LEN;

const _: () = assert!(NAME_REPLY_LEN <= REPLY_MAX);
const _: () = assert!(ID_REPLY_LEN <= REPLY_MAX);

/// 容器坐标那一格的"记"：`0` = 根、`1` = 号（[`Where`] 两种报法在帧里的样子）。
///
/// 根那一路后面那 8 字节**照写零**（不省）：帧是定长的，省了就得再想"读到哪儿算数"。
const AT_ROOT: u8 = 0;
const AT_ID: u8 = 1;

/// 问话里"名字"那一块的起点（`part` / `land`）。
const NAME_AT: usize = 10;
/// 问话里"尾格"那一块的起点（只有 `land` 用）。
const TAIL_AT: usize = NAME_AT + env::wire::NAME_LEN;

// ── 问话：一个动作一条形状 ──────────────────────────────────

/// 一问的荷载——**一个动作一条形状**，没有"报法"那一格可以填错。
///
/// 号那一侧全按 [`EntryId`] 走；名字只出现在两条路上：[`Ask::Road`]（`seek` 收的那条路）
/// 与 `part` / `land` 的**新名**（那是"这一格叫什么"，不是"往哪儿走"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask<'a> {
    /// `seek`：把一条路译成号（**路只出现在这一格**）。
    Road(&'a [Name]),
    /// `list`：列那一块 `Pane` 里的号。
    List(Where),
    /// `part`：在那一块 `Pane` 下，给这个新名分一格。
    Part { at: Where, name: Name },
    /// `land`：在那一块 `Pane` 下，给这个新名落一枚。
    ///
    /// `entry` 是**经会话交出去之后**、种在持树者表里的那一个号（[`ship`] 换回来的），
    /// 不是"客人的 Pie 是几号"——两个编号空间不同源。
    Land {
        at: Where,
        name: Name,
        entry: PieToken,
    },
    /// `find`：那一号后面那一枚 Pie。
    Find(EntryId),
    /// `trim`：把那一号剪掉。
    Trim(EntryId),
    /// `name`：那一号此刻叫什么。
    Name(EntryId),
}

impl Ask<'_> {
    /// 这一问在报文里的**动作码**（与核心那七条原语同名）。
    pub const fn op(&self) -> u8 {
        match self {
            Ask::Road(_) => SEEK,
            Ask::List(_) => LIST,
            Ask::Part { .. } => PART,
            Ask::Land { .. } => LAND,
            Ask::Find(_) => FIND,
            Ask::Trim(_) => TRIM,
            Ask::Name(_) => NAME,
        }
    }
}

/// **解开的一问**（名字已经是 `Name`，故不是借用）。
///
/// 与 [`Ask`] 是一对：编的时候按动作分形状，解的时候也按动作分形状——`op` 与荷载不配
/// （比如 `LAND` 那一码配上一枚号）解不出来，持树者据此答 [`BAD`]。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AskIn {
    /// `seek`：路（最多 [`Operator::ROAD_MAX`] 段）+ **真实段数**（可能超过上限，那一格答 [`FULL`]）。
    Road([Name; Operator::ROAD_MAX], usize),
    /// `list`：容器坐标。
    List(Where),
    /// `part`：容器坐标 + 新名。
    Part {
        at: Where,
        name: Name,
    },
    /// `land`：容器坐标 + 新名 + 入口那一枚。
    Land {
        at: Where,
        name: Name,
        entry: PieToken,
    },
    /// `find` / `trim` / `name`：一枚号（三者的形状一样，故解出来仍是三格）。
    Find(EntryId),
    Trim(EntryId),
    Name(EntryId),
}

/// 把一问编成一帧，返 **`(帧, 实际长度)`**。
///
/// **段数写的是真实条数**（哪怕超过 [`Operator::ROAD_MAX`]）：那样"路太长"由持树者按
/// [`Fail::Full`] 答出来，而不是在这里被悄悄截断成另一条路。
pub fn pack_ask(ask: Ask<'_>) -> ([u8; ASK_MAX], usize) {
    let mut out = [0u8; ASK_MAX];
    out[0] = ask.op();
    let len = match ask {
        Ask::Road(road) => {
            let filled = road.len().min(Operator::ROAD_MAX);
            out[1] = road.len().min(u8::MAX as usize) as u8;
            for (i, name) in road.iter().take(filled).enumerate() {
                let at = 2 + i * env::wire::NAME_LEN;
                out[at..at + env::wire::NAME_LEN].copy_from_slice(name.bytes());
            }
            2 + filled * env::wire::NAME_LEN
        }
        Ask::List(at) => {
            pack_at(&mut out, at);
            10
        }
        Ask::Part { at, name } => {
            pack_at(&mut out, at);
            pack_name_in(&mut out, name);
            42
        }
        Ask::Land { at, name, entry } => {
            pack_at(&mut out, at);
            pack_name_in(&mut out, name);
            out[TAIL_AT..TAIL_AT + 8].copy_from_slice(&entry.to_bytes());
            50
        }
        Ask::Find(id) | Ask::Trim(id) | Ask::Name(id) => {
            out[1..9].copy_from_slice(&id.to_bytes());
            9
        }
    };
    (out, len)
}

/// 把容器坐标写进 `[1 .. 10]`（记 + 号；根那一路的号位照写零）。
fn pack_at(out: &mut [u8; ASK_MAX], at: Where) {
    match at {
        Where::Root => {
            out[1] = AT_ROOT;
            out[2..10].copy_from_slice(&EntryId::new(0).to_bytes());
        }
        Where::At(id) => {
            out[1] = AT_ID;
            out[2..10].copy_from_slice(&id.to_bytes());
        }
    }
}

/// 把新名写进 `[10 .. 42]`。
fn pack_name_in(out: &mut [u8; ASK_MAX], name: Name) {
    out[NAME_AT..NAME_AT + env::wire::NAME_LEN].copy_from_slice(name.bytes());
}

/// 只读第一格**动作码**（空帧 ⇒ `None`：持树者据此答 [`BAD`]，不猜、不崩）。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问：**`op` 决定形状**（见文件头那张表）。**读不懂返 `None`**（持树者据此答 [`BAD`]）。
///
/// 长度为该动作该有的长度是**帧的契约**（[`pack_ask`] 产出的就是那个长度），故短一字节即读不懂。
/// 段数**原样报出去**（哪怕超过上限）：那一格该由持树者答 [`Fail::Full`]——两条都由核心的
/// 判据说了算。
pub fn unpack_ask(op: u8, bytes: &[u8]) -> Option<AskIn> {
    match op {
        SEEK => {
            let count = *bytes.get(1)? as usize;
            let mut road = [Name::EMPTY; Operator::ROAD_MAX];
            // **只解前 `ROAD_MAX` 段**：`pack_ask` 只填了那么多，剩下的段位是零填充——空段不是
            // 名字，拿它去解会把一整帧判成"读不懂"（真机实测：四格全答 `BAD` 就栽在这里）。
            let filled = count.min(Operator::ROAD_MAX);
            for (i, slot) in road.iter_mut().enumerate().take(filled) {
                let at = 2 + i * env::wire::NAME_LEN;
                let raw = bytes.get(at..at + env::wire::NAME_LEN)?;
                let mut buf = [0u8; env::wire::NAME_LEN];
                buf.copy_from_slice(raw);
                *slot = Name::from_bytes(buf).ok()?;
            }
            Some(AskIn::Road(road, count))
        }
        LIST => Some(AskIn::List(unpack_at(bytes)?)),
        PART => Some(AskIn::Part {
            at: unpack_at(bytes)?,
            name: unpack_name(bytes, NAME_AT)?,
        }),
        // 入口那一枚**必须带**：没带（全 0 ⇒ 解不出令牌）就是一句读不懂的帧，不猜。
        LAND => Some(AskIn::Land {
            at: unpack_at(bytes)?,
            name: unpack_name(bytes, NAME_AT)?,
            entry: PieToken::from_bytes(&tail(bytes)?)?,
        }),
        FIND | TRIM | NAME => {
            let id = unpack_id(bytes, 1)?;
            Some(match op {
                FIND => AskIn::Find(id),
                TRIM => AskIn::Trim(id),
                _ => AskIn::Name(id),
            })
        }
        // 没见过的动作码：读不懂（不另立一格）。
        _ => None,
    }
}

/// 解容器坐标（`[1 .. 10]`：记 + 号）。
fn unpack_at(bytes: &[u8]) -> Option<Where> {
    match *bytes.get(1)? {
        AT_ROOT => Some(Where::Root),
        AT_ID => Some(Where::At(unpack_id(bytes, 2)?)),
        _ => None,
    }
}

/// 解一枚号（`[at .. at+8]`，8 字节小端；**不校验"还在不在"**：那一格由核心答）。
fn unpack_id(bytes: &[u8], at: usize) -> Option<EntryId> {
    let raw = bytes.get(at..at + 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(raw);
    Some(EntryId::from_bytes(buf))
}

/// 解一段名字（`[at .. at+32]`；解不出来 ⇒ `None`）。
fn unpack_name(bytes: &[u8], at: usize) -> Option<Name> {
    let raw = bytes.get(at..at + env::wire::NAME_LEN)?;
    let mut buf = [0u8; env::wire::NAME_LEN];
    buf.copy_from_slice(raw);
    Name::from_bytes(buf).ok()
}

/// 解尾格（`[42 .. 50]`）。
fn tail(bytes: &[u8]) -> Option<[u8; 8]> {
    let raw = bytes.get(TAIL_AT..TAIL_AT + 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(raw);
    Some(buf)
}

// ── 答：一串号 / 一枚名字 / 一枚号 ───────────────────────────

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

/// 把一枚号编成一帧答话（写进服务端那只缓冲），返**帧长**（= [`ID_REPLY_LEN`]）。
///
/// 号那一侧**不校验"还在不在"**：持树者答出来的那一枚是刚从树里取的，客侧读到的就是它。
pub fn pack_id(out: &mut [u8; REPLY_MAX], id: EntryId) -> usize {
    out[0] = OK;
    out[1..1 + 8].copy_from_slice(&id.to_bytes());
    ID_REPLY_LEN
}

/// 解开一帧「号」：答话那一格不是 [`OK`] ⇒ `Err(那一格)`。
///
/// **长度必须恰好 9**（对照 [`read_list`]）：短一字节是残帧、长一字节是多出来的东西——
/// 两种都读不懂（[`BAD`]）。
pub fn read_id(bytes: &[u8]) -> Result<EntryId, u8> {
    let Some((&code, body)) = bytes.split_first() else {
        return Err(BAD);
    };
    if code != OK {
        return Err(code);
    }
    if body.len() != 8 {
        return Err(BAD);
    }
    let mut raw = [0u8; 8];
    raw.copy_from_slice(body);
    Ok(EntryId::from_bytes(raw))
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

/// **交出**：把调用方手里那一枚交给持树者（`Accord` 一份副本），返"种在持树者表里"的号；
/// 反过来的那一半（持树者把树上那一枚转授给客人，`find` 的下场）**是同一件事**，故同一个名字
/// ——照实记：这两个方向原先叫 `hang` 与 `give`，收口那一刀并成了这一个。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：持树者查到名字时要**再授出**（`find` 的下场）——
/// 内核那道"持 `VEST` 才交得出去"的闸挡的就是"查到了却授不出去"；拿到它的人可以再传
/// ——那正是"一个名字指向一枚 Pie"的用法，故这里也不替调用方裁剪。
///
/// 身体在 [`crate::session::call::ship`]（**同名的裸手**）；**失败域是本模块的**
/// （`Unknown`）：身体共用，失败值各自说（与 [`map_claim`] / [`map_seat`] 同款）。
pub fn ship(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    crate::session::call::ship(entry, to).map_err(|()| Fail::Unknown)
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────
//
// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）。
pub const ASK_MARK: Mark = Mark::of("ask");

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）。
pub const TIP_MARK: Mark = Mark::of("tip");

/// 提示之路的名字（两侧共用：持树者那侧不用它——它那一枚是自己铸的；引导域用它把
/// 认来的那一枚挂在"名字 → 我手里的一枚"这张账上，好让编排域按名来要）。
pub const TIP_NAME: &str = "operator-tip";
