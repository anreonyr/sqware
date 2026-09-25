//! operator 的**帧那一半** —— 帧、码、记号（内核那几只手的别名与适配在 `protocol` 那一侧的 `call`）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 那一串号的条数对不上 / 表外的码）一格都走不到。拆开之后这一份
//! **只认 `env` / `plan` 与同层 `core`/`judge`**（[`CoordFrame`] 的后半是装配单上的
//! [`Eyes`]），宿主靶能把它逐字编进去跑判据；适配那半（内核手别名、
//! `tree()`、`ship`、两张会话失败域的映射）留在 `call.rs`。
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
//!          land   [0] op  [1] 记    [2 .. 10] 号 [10 .. 42] 名 [42 .. 50] 尾格
//!                 [50] 改   [51] 用   [52 .. 60] 号
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
//! **尾格只剩 `land` 用**：入口那一枚经会话交出去（`ship` 换回来的那个号，不是"客人的 Pie
//! 是几号"），报文里走的只是"种在持树者表里的号"。两个编号空间不同源，互相拿错正是旧树
//! `[33..41]` 那一格的病。
//!
//! **答话有四种形状、各有各的上界**，服务端按 [`REPLY_MAX`] 备一只缓冲。

use env::Mark;
use env::{Name, PieToken, TaskId};
use plan::assembly::Eyes;

use super::core::{EntryId, Fail, Operator, Where};
use super::judge::Id;
// **照实记（同一个词的第二件事）**：本文件里的 `Id` 是 `judge` 的**宽度别名**（u64），
// 与 [`crate::id::Id`]（号的字节面那一枚 trait）同名不同事；trait 只要在作用域里就够用，
// 故按 `_` 引入——不让两个 `Id` 在同一个文件里争一个名字。
use crate::id::Id as _;

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

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格。**前六格与 [`Fail`] 一一对应**，第七格不是失败域
/// 的：这一问读不懂（帧坏了 ⇒ 不猜、不崩）。**第八、九格也不是 [`Fail`]**——那是门外那一问
/// （[`judge`](crate::system::operator::judge)）的两格答案，见 [`DENIED`] / [`UNJUDGED`]。
///
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持树者那一侧编、客人那一侧读）。
pub const UNKNOWN: u8 = 1;
pub const NONEMPTY: u8 = 2;
pub const NOTATILE: u8 = 3;
pub const NOTAPANE: u8 = 4;
pub const FULL: u8 = 5;
pub const DEAD: u8 = 6;
pub const BAD: u8 = 7;
/// **门外那一问答"不"**：这一位不许动这一格。**终态**——换人 / 换目标，别重试。
///
/// **第八格起不再是 [`Fail`] 的对照表**（[`Fail`] 只有六格）：这两格来自适配层的裁决
/// （[`judge`](crate::system::operator::judge)），核心一个字节都不知道它们。分开的理由与
/// [`UNJUDGED`] 同款——"你不许"的下一步与"没铸过 / 剪掉了"不同。
pub const DENIED: u8 = 8;
/// **门外那一问答"判不了"**：这一问要的那条事实问不到——对面不答 / 超时（**会好**），
/// 或那一号是碑 / 那一格是块窗格 / 开者那扇门封印了（**好不了**）。
///
/// 与 [`DENIED`] 分家的理由只有一条，但够硬：**"没资格"与"判不了"是两件事**——混成一格，
/// 就会把"身份服务挂了"读成"我没权限"，整机去查规矩。**它不承诺"等一会儿会好"**：
/// 两类因在客人那一侧是同一个下一步（当趟放弃），把三因分开的是**读数**，不是第三格码。
pub const UNJUDGED: u8 = 9;

/// **落牌的人给这一格声明的条件** —— 两轴，两格。
///
/// ```text
///   [50] 改那一轴   mine: bool        —— 归不归落牌的那一位
///   [51] 用那一轴   标记（0..3）
///   [52 .. 60]      号（8 字节 LE）    —— 只有 1/2/3 那三格用得上
/// ```
///
/// **两轴是两件事**，故各占各的格：
///
/// - **用**那一轴 = [`Rule<Id, Id>`]（[`judge`](super::judge) 那一套四格：公开 / 就是某一位 /
///   在某一位那一支里 / 在某枚盟里）；
/// - **改**那一轴 = 今天原来那一格（"归落牌的那一位"），**它本来就只是 0/1**，故退成一个
///   `bool`——线上值逐字同义（`Owner` 原是 1、`Public` 原是 0）。
///
/// 两轴混成一格就会得出"能改的人自然能用"（而反过来才是常见的那一种）。
///
/// # 这一格原来是一个叫 `Rule` 的两格枚举（照实记：撞名）
///
/// 仓里因此有两个同名的 `Rule`（模型那一侧四格、线上这一侧两格），而持树者那一侧同时
/// `use` 了两个——再加一轴就会写出"这个 `Rule` 不是那个 `Rule`"的代码。这一刀把它拆开：
/// 线上一侧只剩 [`Rule`] 这一个名字（**再出口**自模型那一侧），"改"退成 `bool`。
///
/// **方向也是挑过的**：本文件反向依赖 [`judge`](super::judge)（同一模块树内），而后者从不
/// 依赖本文件——故 [`gate`](super::gate) 那条"不与 `call.rs` 沾边"的纪律一字不破（`call.rs`
/// 拖着 `runtime`，`judge.rs` 不拖）。
pub use super::judge::Rule;

/// 「用那一轴」在帧里的标记。`0` 是公开，也是**兜底**（读不到 / 读不懂都走它）。
const RULE_PUBLIC: u8 = 0;
const RULE_IS: u8 = 1;
const RULE_UNDER: u8 = 2;
const RULE_IN: u8 = 3;
/// `4` 之后的号装的是**格号**（`Rule::Opens`），不是身份号——同一个 8 字节那一格。
const RULE_OPENS: u8 = 4;

/// 把「用那一轴」写进 `[51]`（标记）与 `[52 .. 60]`（号）。
fn pack_rule(out: &mut [u8; ASK_MAX], rule: Rule<Id, Id>) {
    let (tag, id) = match rule {
        Rule::Public => (RULE_PUBLIC, 0),
        Rule::Is(p) => (RULE_IS, p),
        Rule::Under(p) => (RULE_UNDER, p),
        Rule::In(c) => (RULE_IN, c),
        // 格号与身份号同宽（都是 8 字节）⇒ 帧长一个字节都不动。
        Rule::Opens(e) => (RULE_OPENS, e.get() as u64),
    };
    out[TAIL_AT + 9] = tag;
    out[TAIL_AT + 10..TAIL_AT + 18].copy_from_slice(&id.to_le_bytes());
}

/// 由线上那两格还原。**读不懂就不认那条规矩**——按 [`Rule::Public`] 走，而不是把整帧判成
/// 坏（一个陌生/缺失的规矩不该让一句问话变成 [`BAD`]）。**同一句兜底的第二处**：
/// 51 字节之前的老帧读不到这两格，于是逐字同义地回到"公开"。
fn unpack_rule(bytes: &[u8]) -> Rule<Id, Id> {
    let tag = *bytes.get(TAIL_AT + 9).unwrap_or(&RULE_PUBLIC);
    let id = match bytes.get(TAIL_AT + 10..TAIL_AT + 18) {
        Some(raw) => {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(raw);
            Id::from_le_bytes(buf)
        }
        None => 0,
    };
    match tag {
        RULE_IS => Rule::Is(id),
        RULE_UNDER => Rule::Under(id),
        RULE_IN => Rule::In(id),
        RULE_OPENS => Rule::Opens(EntryId::new(id as usize)),
        _ => Rule::Public,
    }
}

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
/// `land` 那一帧的长度：**纯追加**扩到 60（原 51）。
///
/// 前 51 字节一个偏移都没动，两个新格挂在其后——故 51 字节的老帧照旧解得出来（读不到那两格
/// ⇒ 用那一轴按 [`Rule::Public`] 走），而"规矩那一格在最后"这条老纪律也还在。
pub const LAND_FRAME: usize = TAIL_AT + 18;

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
    /// `entry` 是**经会话交出去之后**、种在持树者表里的那一个号（`ship` 换回来的），
    /// 不是"客人的 Pie 是几号"——两个编号空间不同源。
    ///
    /// `rule` 是**落牌的人给这一格声明的"用"那一轴**（[`Rule`]），`mine` 是**"改"那一轴**
    /// （声明归自己之后，别人接手这一格会被拒）。
    Land {
        at: Where,
        name: Name,
        entry: PieToken,
        rule: Rule<Id, Id>,
        mine: bool,
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
    /// `land`：容器坐标 + 新名 + 入口那一枚 + **这一格的两轴条件**
    /// （用那一轴 [`Rule`] / 改那一轴 `mine`）。
    Land {
        at: Where,
        name: Name,
        entry: PieToken,
        rule: Rule<Id, Id>,
        mine: bool,
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
        Ask::Land {
            at,
            name,
            entry,
            rule,
            mine,
        } => {
            pack_at(&mut out, at);
            pack_name_in(&mut out, name);
            out[TAIL_AT..TAIL_AT + 8].copy_from_slice(&entry.to_bytes());
            // **两轴各一格**：`[50]` 是原来那一格（值逐字同义），`[51]` 起是纯追加。
            out[TAIL_AT + 8] = mine as u8;
            pack_rule(&mut out, rule);
            LAND_FRAME
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
        // 两轴那两格**可以没有**（60 字节之前的老帧）：`mine` 读 `[50]`（51 字节的老帧本来
        // 就有它，值逐字同义），用那一轴读不到 ⇒ [`Rule::Public`]——帧加格子不该让旧调用方
        // 当场变坏，也不该把一个陌生的标记读成"这一问读不懂"。
        LAND => Some(AskIn::Land {
            at: unpack_at(bytes)?,
            name: unpack_name(bytes, NAME_AT)?,
            entry: PieToken::from_bytes(&tail(bytes)?)?,
            rule: unpack_rule(bytes),
            mine: *bytes.get(TAIL_AT + 8).unwrap_or(&0) != 0,
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
///
/// **照实记（为什么不与 `coalition` 的 [`Window`](crate::system::coalition::core::Window) 并成一个容器）**：
/// 两者都在搬"一串号"，差的正是**"未完"那一格**——盟籍**没有上限**（一格盟可以很多人）⇒ 那边
/// 必须带 `more`，并因此把格子存成 `[Option<T>; CAP]`（泛型 + `const new` 造不出 `T` 的占位，
/// 而零号是**真格子**，不能拿它当空）；**一条 pane 本来就有顶**（[`Operator::PANE_CAP`]）⇒
/// "还没完"这件事在这一族**不存在**，带 `more` 就是一格**恒假**的字段。故两处各留一个，
/// **帧形也跟着**（`LIST_REPLY_LEN` 无"未完"、`SEQ_REPLY_LEN` 有）。
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

    /// 按号序（就是帧里的次序）走一遍。
    ///
    /// **照实记（这一份只剩这一个读面）**：原先还有 `len` / `is_empty` / `get` 三格——
    /// `is_empty` / `get` **全仓零用家**，`len` 只被宿主靶的 `judge` 靶用过（`back.len()`），
    /// 而生产路径（`echo` 的读数、`pack_list`）与靶都只走 `iter()` ⇒ 三格都删掉，靶里那一处
    /// 改写成 `iter().count()`。要"几枚"就问这一句。
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

/// 把**回信孔那一格**编成一帧答话（`find` 的下场）：与 [`pack_id`] **同形不同物**——
/// 都是 `[OK][8 字节]`（[`ID_REPLY_LEN`]），但那一枚号是"我给你的那一枚**在你表里**是几号"
/// （`PieToken`），不是 `EntryId`。故**另起一名、不复用** `pack_id`：两枚号类型不同，混用
/// 就是把"树的坐标"与"你表里的门闩"当成一件事。
///
/// **照实记（这一格为什么在帧里）**：从前 `find` 只答一格状态，客人拿到 `OK` 之后还得**扫
/// 自己的表**按"谁给的"把那一枚认回来（`operator::take`）。而号本来就在持树者手上
/// ——`port::ship` 的 `to.seed()`，原先被 `.map(|_| ())` 扔掉——故随答话一起过来，
/// 客人拿它一次 `Reserve` 就验得完。代价照实记：**答话丢了一趟，那一枚号也跟着丢**（今天
/// 还能靠扫表侥幸认回来）——与 rtc / principal / coalition 那三面同一个取舍。
pub fn pack_seed(out: &mut [u8; REPLY_MAX], seed: PieToken) -> usize {
    out[0] = OK;
    out[1..1 + 8].copy_from_slice(&seed.to_bytes());
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

// ── 协调那一帧（**装配者 → 持树者**，不是门外那一问）──────────
//
// 它不在上面那张图里：上面那几帧是**客人 ↔ 持树者**的一问一答，这一条是**装配者递过来的
// 一格号**（装完那一位域之后一次）。两族同住本文件，因为"帧形只有一处"这一条不分装配期与
// 运行期——它是同一棵树的两半。

env::frame! {
    /// **协调那一帧**——装配者告诉持树者"哪一位域把门牌交过来了、它是哪一双眼睛"。
    ///
    /// 布局（由字段表求和得出，**这里不再写数**）：那一位域自己的号（`TaskId`）｜[`Eyes`]。
    ///
    /// **照实记（后 8 字节的对齐方式换过一次）**：原先这一枚枚举（`Role`）与持树者那一侧的
    /// `ROLE_ROSTER` / `ROLE_LEAGUE` 常量**各写一遍** 0/1，靠两边注释说"必须同值"。现在两侧共读
    /// [`Eyes`]（`plan::assembly`）——装配单上那一格、这一帧、收的那一侧，一处定义。
    ///
    /// **照实记（后 8 字节的来历）**：门禁那一刀里它们是**保留零**。这一刀起有了意思——于是两枚
    /// 门牌可以**分两帧、按位递**，"长度即语义"（16 = 这一帧）一个字没破。
    ///
    /// **照实记（三个名字并成一个，再并成一张表）**：收的那一侧原先自己写着 `COORD_FRAME = 16`，
    /// 靠注释说"必须同值"——同一条长度写两处，改一处漏一处**编得过**，症状要等帧被读成"读不懂"
    /// 才显形（正是 [`Eyes`] 那一段照实记里同一个毛病的第二次）。先把两个常量并成一个，再并成上面
    /// 这张字段表：`16` 这个数从此**一处都不写**，两侧读的是同一个 `LEN`；后 8 字节怎么翻也不再由
    /// 装配侧按 `Eyes::of_wire` 现算、由收侧按 `from_le_bytes` 现翻——那是 [`Eyes`] 自己的 `Field`
    /// （读不懂 ⇒ 整帧读不懂，收侧照旧报一句）。
    ///
    /// **照实记（它为什么从 `programs/.../operator/bridge.rs` 搬到这里）**：那一帧原先跟着它的
    /// 那个常量住在**装配侧**（`pub(crate) const COORD_FRAME`），故它**只有真机能跑**——宿主靶
    /// 编不到 `programs`。搬进「约」的这一半之后，它与 [`Tip`](crate::system::board::frame::Tip)
    /// 一样在宿主上编得动，"表外的眼睛码 ⇒ 整帧读不懂"那一条因此有了判据
    /// （`crates/protocol-case/tests/judge.rs`）。
    ///
    /// # 为什么门牌不由装配者转授（照实记：这一格返工过）
    ///
    /// 第一版让装配者把那一枚门牌**再转授**给树。真机栽了：`principal` 那一格报
    /// `operator:coord-ship`，内核答 `-1 Denied`——而装配者手里那一枚权限位是对的（`0x3`：
    /// `FETCH|STORE`；**那一版授的是这两位**，后来收成 `STORE`——见 `principal/server.rs` 给生我者
    /// 那一格的照实记）、也在表里。那一格的三道闸（覆盖子集 / 持 `VEST` / 形态一致）都不是原因，
    /// 于是这一笔"第二手转授"在装配窗口里带进了说不清的锚与来历问题。
    ///
    /// **改成由各域自己交**（它本来就是树的客人：`serve_tree` 那一趟已经握着树路）：
    /// 它 `serve_tree` 之后把门牌那一枚直接 `ship` 给持树者，再把**自己的号 + 哪一双眼睛**经这一
    /// 帧递过去。于是：
    ///
    /// - 持树者拿到的门牌**一手来源**，没有第二手转授；
    /// - 装配者只剩"递一格号"这一件事，`attach` 里不多一次 `Ship`；
    /// - 各域本来就与树有一条会话（挂门牌那一趟），这一笔是它的近邻。
    pub struct CoordFrame {
        who: TaskId,
        eyes: Eyes,
    }
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::NonEmpty => NONEMPTY,
    Fail::NotATile => NOTATILE,
    Fail::NotAPane => NOTAPANE,
    Fail::Full => FULL,
    Fail::Dead => DEAD,
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────
//
// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）。
///
/// **带面名**（同 `board` 那面的 `board-ask`、以及两面的 `*-tip`）：问话孔的认领键是
/// "**谁开的 + 记号**"，而**同一枚任务可能同时是两面的客人**（`echo` / `guest` / `principal`
/// …都是：一边问板、一边问树）——两枚孔都铸在**它自己那张表**里，记号再一样就分不开了。
///
/// 照实记（这一格是**量出来的**，不是想出来的）：把记号统一成 `ask` 之后，客侧"先找后铸"
/// 的那一手当场把**板那一枚**当成了树那一枚交回来 ⇒ 树那条路永远没有问话孔 ⇒ 装机就塌
/// （实测 `principal: tree … got=false` + `system: service failed`，`examine` **0/3**）。
/// 今天两面的孔落进**两张不同的表**（板线程 / 持树者），故这件事从来没露过头。
pub const ASK_MARK: Mark = Mark::of("operator-ask");

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）。
pub const TIP_MARK: Mark = Mark::of("tip");

/// 提示之路的名字（两侧共用：持树者那侧不用它——它那一枚是自己铸的；引导域用它把
/// 认来的那一枚挂在"名字 → 我手里的一枚"这张账上，好让编排域按名来要）。
pub const TIP_NAME: &str = "operator-tip";

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台的一条运行时用例（`the_operator_marks_do_not_collide_with_the_other_doors`，
// 搬出运行时源时随用例一起改到这里）：这几枚值各是一枚 FNV 散列（`env::Mark::of`），
// **撞了就是那次装机塌掉**（见上面 `ASK_MARK` 的照实记）。挪到编译期之后，riscv 那一档也一样
// 钉着——"一漂就编不过"，且不再占一条用例。
//
// 比的是 `.get()` 那个裸值：`Mark` 的 `PartialEq` 不是 `const`，而 `get` 是 `const fn`。
const _: () = assert!(ASK_MARK.get() != Mark::of("board-ask").get());
const _: () = assert!(ASK_MARK.get() != Mark::of("ask").get());
const _: () = assert!(ASK_MARK.get() != TIP_MARK.get());
const _: () = assert!(TIP_MARK.get() != Mark::of(TIP_NAME).get());
