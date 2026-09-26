//! operator 的**帧那一半** —— 帧、码、记号（内核那几只手的别名与适配在 `protocol` 那一侧的 `mod.rs`）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 那一串号的条数对不上 / 表外的码）一格都走不到。拆开是为了让那些
//! 边角在**宿主靶**上编得动；**那台靶已删**（用户裁定"protocol-case 没必要"）⇒ 这一份照旧只认
//! `env` / `plan` 与同层 `core`/`judge`（[`CoordFrame`] 的后半是装配单上的 [`Eyes`]），而那些
//! 边角今天**没有判据**；适配那半（内核手别名、`tree()`、`ship`）留在 `protocol` 那一侧的
//! `mod.rs`，两张会话失败域的映射随本层 [`core`](super::core) 同住。
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
//!   Req    Road   [0] op  [1] 段数  [2 .. 2+32k] 路                （k ≤ ROAD_MAX）
//!          List   [0] op  [1] 记    [2 .. 10]     号              （记：0 = 根 / 1 = 号）
//!          Part   [0] op  [1] 记    [2 .. 10]     号  [10 .. 42] 名
//!          Land   [0] op  [1] 记    [2 .. 10] 号 [10 .. 42] 名 [42 .. 50] 尾格
//!                 [50] 改   [51] 用   [52 .. 60] 号
//!          Find   [0] op  [1 .. 9] 号
//!          Trim   同 Find
//!          Name   同 Find
//!   Union    [0] status                                    —— 一格的答
//!          [0] status   [1] 条数   [2 ..] 号             —— 列
//!          [0] status   [1 ..] 名字                       —— 名（长度即名长）
//!          [0] status   [1 .. 9] 号                       —— 号（`land` / `part` / `seek`，定长 9）
//! ```
//!
//! **答那一侧四种形状在线上分不开**（都以状态那一格起头，而"名"那一条是变长的：**长度即
//! 名长**）⇒ 收进来的那一面是**原样的字节**（[`Said`]），由**问的人**按自己问的那一条读；
//! 编的那一面是 [`Union`]（五种编法：一格状态 / 一串号 / 一枚名字 / 坐标 / 门闩）。
//!
//! **问话一个动作一条形状**（不再是"一帧定长、尾格含义由 op 定"）：荷载收什么，帧里就写什么
//! ——没有一个"报法"字段可以填错，也没有第二个意思可读。最长的仍是 `Road` 那一条
//! （[`REQ_LEN`]，路封顶 [`Operator::ROAD_MAX`] 段），其余都落在十到五十字节。
//!
//! **每一张形状一张字段表**（[`RoadHead`] / [`List`] / [`Part`] / [`Land`] / [`Entry`]）：
//! 偏移一处都不写。**照实记（表名的口径收窄了一次）**：板那一族的表按**荷载**起名（那一族
//! 有两个动作共用一张）；这一族**一条问一张表**，只有那三条只报号的（`find` / `trim` /
//! `name`）共用——那一张按荷载叫 [`Entry`]（它是唯一一处"两个名字落在同一张表上"）。
//!
//! **变长那一条只有 `Road`**：它由 [`RoadHead`]（头两格）与 [`env::wire::store_tail`] /
//! [`env::wire::fetch_tail`]（路那一段）拼成——**族里没有 `2 + i * 32` 这种句子**（用户裁定：
//! 尾巴不许手写）。
//!
//! **尾格只剩 `land` 用**：入口那一枚经会话交出去（`ship` 换回来的那个号，不是"客人的 Pie
//! 是几号"），报文里走的只是"种在持树者表里的号"。两个编号空间不同源，互相拿错正是旧树
//! `[33..41]` 那一格的病。
//!
//! **答话有四种形状、各有各的上界**，船台那只缓冲按 [`UNION_LEN`] 备（最大那一形）。

use env::Mark;
use env::{Name, PieToken, TaskId};
use plan::assembly::Eyes;

use super::core::judge::Id;
use super::core::{EntryId, Fail, Operator, Where};
// **照实记（同一个词的第二件事）**：本文件里的 `Id` 是 `judge` 的**宽度别名**（u64），
// 与 [`crate::id::Id`]（号的字节面那一枚 trait）同名不同事；trait 只要在作用域里就够用，
// 故按 `_` 引入——不让两个 `Id` 在同一个文件里争一个名字。
use crate::id::Id as _;
use crate::message::Message;

// ── 码 ──────────────────────────────────────────────────────

// 七个动作在报文里的码——**与核心那七条原语同名**（`land` / `part` / `find` / `trim` /
// `list` / `seek` / `name`）：线上与模型是同一件事的两层，不该各起一套词。
//
// **它们不再是协议面**（照板那一族的先例）：编的那一侧由 [`Req`] 说、解的那一侧由 [`Wire`]
// 说，每一枚码各被读一次（字段表头一格 `op`）。外面认的是类型 ⇒ 降为私有——没有读者的格不
// 留在面上。
const LAND: u8 = 1;
const PART: u8 = 2;
const FIND: u8 = 3;
const TRIM: u8 = 4;
const LIST: u8 = 5;
const NAME: u8 = 6;
// **第七个动作**：把一条路**译成号**——名字只能走到这一格，往下一律按号。
//
// 数字取 7 是白捡的：答话那一列里 `BAD` 也是 7，但**动作码与答话码本来就是两张表**
// （今天 `LAND`..`NAME` 的 1..6 与 `UNKNOWN`..`DEAD` 的 1..6 已经重号），故两边各按各的序列。
const SEEK: u8 = 7;

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格。**前六格与 [`Fail`] 一一对应**，第七格不是失败域
/// 的：这一问读不懂（帧坏了 ⇒ 不猜、不崩）。**第八、九格也不是 [`Fail`]**——那是门外那一问
/// （[`judge`](crate::system::operator::core::judge)）的两格答案，见 [`DENIED`] / [`UNJUDGED`]。
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
/// （[`judge`](crate::system::operator::core::judge)），核心一个字节都不知道它们。分开的理由与
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
/// - **用**那一轴 = [`Rule<Id, Id>`]（[`judge`](super::core::judge) 那一套四格：公开 / 就是某一位 /
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
/// **方向也是挑过的**：本文件反向依赖 [`judge`](super::core::judge)（同一模块树内），而后者从不
/// 依赖本文件——故 [`gate`](super::core::gate) 那条"不与 `protocol` 那一侧沾边"的纪律一字不破（那一侧
/// 拖着 `runtime`，`judge.rs` 不拖）。
pub use super::core::judge::Rule;

/// 问话那一侧的上界：**最长那一条**（`Road`：`op` ＋ 段数 ＋ [`Operator::ROAD_MAX`] 段名字）。
///
/// 服务端按它备一只缓冲（收下来的帧不会超过它），各条问话的**实际**长度由形状说——定长那几条
/// 是字段表求和（`LEN`），`Road` 那一格是 [`env::wire::store_tail`] 交回的游标。
pub const REQ_LEN: usize = RoadHead::LEN + Operator::ROAD_MAX * env::wire::NAME_LEN;

/// 一答的**上限**：四种答形里最大的那一形（`[status][条数][号…]`）。一条 `Pane` 本来就不超过
/// [`Operator::PANE_CAP`] 枚 ⇒ **一趟答得完，没有"未完"那一格**（对照 `coalition` 那一侧：盟籍
/// 没有上限，故那里必须带一格"未完"）。
///
/// 船台那只缓冲就是它（[`Message::Buf`]）；另两形都短于它——编译期钉住（`名` 那一形最长是
/// 状态 ＋ `NAME_LEN - 1` 个字节，`号` 那一形是状态 ＋ 8）。
pub const UNION_LEN: usize = 2 + Operator::PANE_CAP * 8;

const _: () = assert!(Status::LEN + (env::wire::NAME_LEN - 1) <= UNION_LEN);
const _: () = assert!(Status::LEN + <[u8; 8] as env::wire::Field>::WIDTH <= UNION_LEN);

// 那几枚偏移常量（`AT_ROOT` / `AT_ID` / `NAME_AT` / `TAIL_AT` / `LAND_FRAME`）随字段表一起退场：
// "记"归 [`Where`] 自己的 `Field`（它住 `core.rs`——impl 跟着类型走），其余几个数由各张表求和
// 得出（`Land::LEN` = 60、`Part::LEN` = 42 …），而 `LAND_FRAME` 那个名字没有读者了。
//
// **照实记（`land` 的长度契约收紧了）**：从前 50 字节起就收——最后那两轴读不到就按
// [`Rule::Public`] 走（那是给"还没写这两轴的调用方"留的兜底）。字段表把长度变成**契约**：
// `Land` 就是 60 字节，短一字节整帧读不懂。仓里没有第二种长度（编那一侧一律写全）。

// ── 问话：一个动作一条形状，一张形状一张字段表 ──────────────

/// `Road` 那一问的**头两格**：动作码 ＋ **段数**。
///
/// **段数写的是真实条数**（哪怕超过 [`Operator::ROAD_MAX`]）：那样"路太长"由持树者按
/// [`Fail::Full`] 答出来，而不是在这里被悄悄截断成另一条路。故这一格**允许大于实际带的
/// 项数**——它是**声明**，不是长度（"尾巴"那一族里只有它这样）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct RoadHead {
    pub op: u8,
    pub count: u8,
}

/// `List` 那一问：动作码 ＋ 容器坐标。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct List {
    pub op: u8,
    pub at: Where,
}

/// `Part` 那一问：动作码 ＋ 容器坐标 ＋ 新名。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Part {
    pub op: u8,
    pub at: Where,
    pub name: Name,
}

/// `Land` 那一问：动作码 ＋ 容器坐标 ＋ 新名 ＋ 入口那一枚 ＋ **这一格的两轴条件**
/// （改那一轴 `mine` / 用那一轴 `rule`）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Land {
    pub op: u8,
    pub at: Where,
    pub name: Name,
    pub entry: PieToken,
    pub mine: bool,
    pub rule: Rule<Id, Id>,
}

/// `Find` / `Trim` / `Name` 那三问**共用**的形状：动作码 ＋ 一枚号。
///
/// **照实记（这三条为什么共用一张表）**：三者的荷载逐字同形（一枚 [`EntryId`]），差别只在
/// 动作码那一格——故解出来仍是三格（[`Wire::Find`] / [`Wire::Trim`] / [`Wire::Name`]），
/// 而"这一格占多宽"只有一处。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub op: u8,
    pub id: EntryId,
}

/// **一问的荷载**——一个动作一条形状，没有"报法"那一格可以填错。
///
/// 号那一侧全按 [`EntryId`] 走；名字只出现在两条路上：[`Req::Road`]（`seek` 收的那条路）
/// 与 `part` / `land` 的**新名**（那是"这一格叫什么"，不是"往哪儿走"）。
///
/// **照实记（名字）**：这一族从前叫 `Ask`（收的那一面叫 `AskIn`）。用户裁定 `Ask` / `Reply`
/// 那一套不要，用 **`Req` / `Wire` / `Union`**——故这里是新生的名字，不是改名。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req<'a> {
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

/// **解开的一问**（名字已经是 `Name`，故不是借用）。
///
/// 与 [`Req`] 是一对：编的时候按动作分形状，解的时候也按动作分形状——`op` 与荷载不配
/// （比如 `LAND` 那一码配上一枚号）解不出来，持树者据此答 [`BAD`]。
///
/// **照实记（它为什么与 [`Req`] 是两个类型）**：`Road` 那一格编的时候借一条路（`&[Name]`），
/// 解出来是**自己那一份**（`[Name; ROAD_MAX]` ＋ 真实段数）——两种形状本来就不一样。
/// **表外的动作码不另立一格**（与板那一族不同）：树这一侧对它答 [`BAD`]，故解不出来就是
/// `None`（见 [`Message::fetch`] 那一段）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
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

impl Message for Req<'_> {
    type In = Wire;
    /// 这一族的缓冲：**最长那一条**（[`REQ_LEN`]）。
    type Buf = [u8; REQ_LEN];
    const EMPTY: Self::Buf = [0u8; REQ_LEN];

    /// 编进 `out`：**动作码由形状给**（不在别处再写一遍），偏移与长度由字段表求和。
    ///
    /// 唯一的例外是 `Road` 那一格的**尾巴**（路）：头两格归 [`RoadHead`]，路那一段交给
    /// [`env::wire::store_tail`]——两处都不写偏移。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match *self {
            Req::Road(road) => {
                let filled = road.len().min(Operator::ROAD_MAX);
                let head = RoadHead {
                    op: SEEK,
                    count: road.len().min(u8::MAX as usize) as u8,
                };
                head.store_in(out)?;
                env::wire::store_tail(out, RoadHead::LEN, &road[..filled])
            }
            Req::List(at) => List { op: LIST, at }.store_in(out),
            Req::Part { at, name } => Part { op: PART, at, name }.store_in(out),
            Req::Land {
                at,
                name,
                entry,
                rule,
                mine,
            } => Land {
                op: LAND,
                at,
                name,
                entry,
                mine,
                rule,
            }
            .store_in(out),
            Req::Find(id) => Entry { op: FIND, id }.store_in(out),
            Req::Trim(id) => Entry { op: TRIM, id }.store_in(out),
            Req::Name(id) => Entry { op: NAME, id }.store_in(out),
        }
    }

    /// 解开一问：**`op` 决定形状**（见文件头那张表）。**读不懂返 `None`**（持树者据此答
    /// [`BAD`]）。
    ///
    /// **长度为该形状该有的长度是帧的契约**（各张表的 `LEN`，`store` 产出的就是那个长度），
    /// 故短一字节、长一字节都读不懂。**段数原样报出去**（哪怕超过上限）：那一格该由持树者答
    /// [`Fail::Full`]——两条都由核心的判据说了算。
    ///
    /// **只解前 `ROAD_MAX` 段**：编的那一侧只填了那么多，剩下的段位是零填充——空段不是名字，
    /// 拿它去解会把一整帧判成"读不懂"（真机实测：四格全答 `BAD` 就栽在这里）。
    ///
    /// **表外的动作码 ⇒ `None`**：树这一族不另立"表外的码"那一格（对它的答话与"读不懂"
    /// 同一句，见 [`Wire`] 的照实记）。
    fn fetch(bytes: &[u8]) -> Option<Wire> {
        let op = *bytes.first()?;
        Some(match op {
            SEEK => {
                let head = RoadHead::fetch(bytes)?;
                let count = head.count as usize;
                // **只解前 `ROAD_MAX` 段**：编的那一侧只填了那么多，剩下的段位是零填充——空段
                // 不是名字，拿它去解会把一整帧判成"读不懂"（真机实测：四格全答 `BAD` 就栽在
                // 这里）。**长度也是形状的一部分**：`2 ＋ 填进去的段数 × 32`。
                let filled = count.min(Operator::ROAD_MAX);
                if bytes.len() != RoadHead::LEN + filled * env::wire::NAME_LEN {
                    return None;
                }
                let mut road = [Name::EMPTY; Operator::ROAD_MAX];
                env::wire::fetch_tail(bytes, RoadHead::LEN, &mut road[..filled])?;
                Wire::Road(road, count)
            }
            LIST if bytes.len() == List::LEN => Wire::List(List::fetch(bytes)?.at),
            PART if bytes.len() == Part::LEN => {
                let at = Part::fetch(bytes)?;
                Wire::Part {
                    at: at.at,
                    name: at.name,
                }
            }
            LAND if bytes.len() == Land::LEN => {
                let at = Land::fetch(bytes)?;
                Wire::Land {
                    at: at.at,
                    name: at.name,
                    entry: at.entry,
                    rule: at.rule,
                    mine: at.mine,
                }
            }
            FIND | TRIM | NAME if bytes.len() == Entry::LEN => {
                let id = Entry::fetch(bytes)?.id;
                match op {
                    FIND => Wire::Find(id),
                    TRIM => Wire::Trim(id),
                    _ => Wire::Name(id),
                }
            }
            // 没见过的动作码、或长度不是这张形状该有的那个 ⇒ 读不懂（不另立一格）。
            _ => return None,
        })
    }
}

// 手写的那六手（`op_of` / `unpack_ask` / `unpack_at` / `unpack_id` / `unpack_name` / `tail`）与
// `pack_ask` 一起退场：编与解各由"一张字段表 ＋ 一条 `match`"说（见上面那一段）。
//
// **照实记（那六手都是"同一件事的第二处"）**：`unpack_at` 与 `pack_at` 各写一遍"记 ＋ 号"、
// `unpack_name` 与 `pack_name_in` 各写一遍"名字那一格怎么切"、`pack_rule` 与 `unpack_rule` 各
// 写一遍那九个字节——写者与读者分居文件两头，**错一处编得过**，症状要等那一帧被读成"读不懂"
// 才显形。今天这三件事各只有一处：`Where` / `Name` / `Rule` 各自的 `Field`。

// ── 答：一格状态 / 一串号 / 一枚名字 / 一枚号 ─────────────────

/// 一帧「列」的读数：号最多 [`Operator::PANE_CAP`] 枚。
///
/// **照实记（为什么不与 `coalition` 的 [`Window`](crate::system::coalition::core::Window) 并成一个容器）**：
/// 两者都在搬"一串号"，差的正是**"未完"那一格**——盟籍**没有上限**（一格盟可以很多人）⇒ 那边
/// 必须带 `more`，并因此把格子存成 `[Option<T>; CAP]`（泛型 + `const new` 造不出 `T` 的占位，
/// 而零号是**真格子**，不能拿它当空）；**一条 pane 本来就有顶**（[`Operator::PANE_CAP`]）⇒
/// "还没完"这件事在这一族**不存在**，带 `more` 就是一格**恒假**的字段。故两处各留一个，
/// **帧形也跟着**（[`Tally`] 无"未完"、coalition 的 `SeqHead` 有）。
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

    /// 收一串（**收够 [`Operator::PANE_CAP`] 枚就停**：一条 pane 本来就不超过它）。
    ///
    /// **照实记（它替掉了 `pack_list` 那一手）**：从前编那一侧直接往缓冲里写（`2 + n * 8`
    /// 那几个偏移）；现在编的是**这一枚容器**，落字节归 [`Tally`] 与 [`env::wire::store_tail`]。
    pub fn of(ids: impl Iterator<Item = EntryId>) -> Listing {
        let mut listing = Listing::new();
        for id in ids.take(Operator::PANE_CAP) {
            listing.push(id);
        }
        listing
    }

    /// 按号序（就是帧里的次序）走一遍。
    ///
    /// **照实记（这一份只剩这一个读面）**：原先还有 `len` / `is_empty` / `get` 三格——
    /// `is_empty` / `get` **全仓零用家**，`len` 只被宿主靶用过（`back.len()`），而生产路径
    /// （`echo` 的读数）只走 `iter()` ⇒ 三格都删掉，那处改写成 `iter().count()`。
    /// 要"几枚"就问这一句。
    pub fn iter(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.ids[..self.n].iter().copied()
    }

    /// 那一段号——**编那一侧要它**（`store_tail` 走的是一条切片，不是一个迭代器）。
    pub fn as_slice(&self) -> &[EntryId] {
        &self.ids[..self.n]
    }

    /// 收一枚。**满了就丢**：一条 pane 本来就不超过 [`Operator::PANE_CAP`] 枚。
    fn push(&mut self, id: EntryId) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = id;
            self.n += 1;
        }
    }
}

// ── 答那一侧的三张字段表（四种答形共用它们）──────────────────

/// **头一格**：状态。它自己就是"一格状态"那一形（六格失败与"门外那两格"都走它），也是另外
/// 三形的起头。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「列」那一形的**头两格**：状态 ＋ **条数**（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
///
/// **这一格的条数与帧长绑死**（读的人两边对不上就判读不懂），故它**不是** `Road` 那一格
/// 的条数（那里的条数是**声明**，允许大于实际带的）——两句不同的话，故各说各的。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tally {
    pub status: u8,
    pub count: u8,
}

/// 「号」那一形：`[status][8 字节]`——**定长 9**（`part` / `seek` 答坐标、`find` 答门闩，
/// 线上逐字同形）。
///
/// **照实记（这一格的类型为什么是裸 8 字节）**：两个号空间（[`EntryId`] / [`PieToken`]）
/// 在这一格上分不开，故字段表不假装它是哪一枚——读面见 [`Said::entry`] / [`Said::seed`]。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Word {
    pub status: u8,
    pub word: [u8; 8],
}

/// **一答的形状**——答有四种：一格状态 / 一串号 / 一枚名字 / 一枚号。
///
/// **照实记（名字）**：这一族从前是 `pack_list` / `pack_name` / `pack_id` / `pack_seed` 四枚
/// 自由函数（外加 `read_list` / `read_name` / `read_id` 三枚）。用户裁定这一族用
/// `Req` / `Wire` / `Union`，而答的**读**那一面叫 [`Said`]——故这里是新生的名字，不是改名。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Union {
    /// 一格状态（成功 / 六格失败 / 门外那两格）——**没有任何荷载**。
    Status(u8),
    /// `list` 的下场：一串号。
    List(Listing),
    /// `name` 的下场：一枚名字（**长度即名长**）。
    Name(Name),
    /// `part` / `seek` 的下场：那一格**坐标**。
    Entry(EntryId),
    /// `find` 的下场：那一格是"我给你的那一枚**在你表里**是几号"（[`PieToken`]）。
    ///
    /// **与 [`Union::Entry`] 同形不同物**（都是 `[OK][8 字节]`）而**另起一格、不复用**：两枚号
    /// 类型不同，混用就是把"树的坐标"与"你表里的门闩"当成一件事。
    ///
    /// **照实记（这一格为什么在帧里）**：从前 `find` 只答一格状态，客人拿到 `OK` 之后还得**扫
    /// 自己的表**按"谁给的"把那一枚认回来（`operator::take`）。而号本来就在持树者手上
    /// ——`port::ship` 的 `to.seed()`，原先被 `.map(|_| ())` 扔掉——故随答话一起过来，客人拿它
    /// 一次 `Reserve` 就验得完。代价照实记：**答话丢了一趟，那一枚号也跟着丢**（今天还能靠扫表
    /// 侥幸认回来）——与 rtc / principal / coalition 那三面同一个取舍。
    Seed(PieToken),
}

/// **收进来的一答**：**原样的字节** ＋ 四个读法。
///
/// **照实记（答这一侧为什么不像问那一侧那样"一个类型说形状"）**：四种答形**在线上分不开**
/// ——`[OK][条数][号…]`、`[OK][名字]`、`[OK][8 字节]` 都以状态那一格起头，而"名"那一条是变长的
/// （**长度即名长**：没有终止符、也没有条数）。分得开它们的是**问的人**——他问的是哪一条自己
/// 知道。故这一枚把字节原样收下，四个读法各按一形解；**形状不对 ⇒ [`BAD`]**（与从前那三枚
/// `read_*` 同一个判据，只是收在了一处）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    buf: [u8; UNION_LEN],
    len: usize,
}

impl Said {
    /// 这一条答的字节。
    fn bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or(&[])
    }

    /// 那一格状态（四种答形的头一格都是它）。
    ///
    /// 空帧（连状态都没有）⇒ [`BAD`]——核里空不是消息，故这一格只防"读法被用错"。
    pub fn code(&self) -> u8 {
        self.bytes().first().copied().unwrap_or(BAD)
    }

    /// 按「号」那一形读（`land` / `part` / `seek` 的下场）：`[status][8 字节]` → **坐标**。
    ///
    /// 状态不是 [`OK`] ⇒ `Err(那一格码)`；不是那一形（长度不对）⇒ `Err(BAD)`。
    pub fn entry(&self) -> Result<EntryId, u8> {
        Ok(EntryId::from_bytes(self.word()?))
    }

    /// 按「门闩」那一形读（`find` 的下场）：同一形状 → **你表里的那一枚号**。
    pub fn seed(&self) -> Result<PieToken, u8> {
        PieToken::from_bytes(&self.word()?).ok_or(BAD)
    }

    /// 「号」那一形里的那 8 字节（上面两个读法共用的那一格）。
    fn word(&self) -> Result<[u8; 8], u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        if bytes.len() != Word::LEN {
            return Err(BAD);
        }
        Ok(Word::fetch(bytes).ok_or(BAD)?.word)
    }

    /// 按「名」那一形读（`name` 的下场）：`[status][名字]` → 一枚名字。
    ///
    /// 名字读不懂（空 / 太长 / 含 NUL / 不是 UTF-8）⇒ `Err(BAD)`：`Name` 那一侧的四格失败域
    /// 在这里**归一格**——问的人能做的补救是同一件（这一帧坏了，重问）。
    pub fn name(&self) -> Result<Name, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let text = env::wire::fetch_bytes(bytes, Status::LEN).ok_or(BAD)?;
        Name::from_slice(text).map_err(|_| BAD)
    }

    /// 按「列」那一形读（`list` 的下场）：`[status][条数][号…]` → 一串号。
    ///
    /// **帧长即条数**：条数与剩下那些字节对不上（或条数超过 [`Operator::PANE_CAP`]）⇒
    /// `Err(BAD)`——短一字节也是它。
    pub fn list(&self) -> Result<Listing, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let head = Tally::fetch(bytes).ok_or(BAD)?;
        let count = head.count as usize;
        if count > Operator::PANE_CAP {
            return Err(BAD);
        }
        let body = bytes.get(Tally::LEN..).ok_or(BAD)?;
        let mut ids = [EntryId::new(0); Operator::PANE_CAP];
        let end = env::wire::fetch_tail(body, 0, &mut ids[..count]).ok_or(BAD)?;
        if end != body.len() {
            return Err(BAD);
        }
        Ok(Listing::of(ids[..count].iter().copied()))
    }
}

impl Message for Union {
    /// 收的那一面是 [`Said`]（**原样的字节**——形状由问的人认，见它的照实记）。
    type In = Said;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    /// 编进 `out`：状态由形状给（不在别处再写一遍），变长那两段交给 `env::wire` 的两个尾巴。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match *self {
            Union::Status(code) => Status { status: code }.store_in(out),
            Union::List(list) => {
                let ids = list.as_slice();
                let head = Tally {
                    status: OK,
                    count: ids.len() as u8,
                };
                head.store_in(out)?;
                env::wire::store_tail(out, Tally::LEN, ids)
            }
            Union::Name(name) => {
                let at = Status { status: OK }.store_in(out)?;
                env::wire::store_bytes(out, at, name.text())
            }
            Union::Entry(id) => Word {
                status: OK,
                word: id.to_bytes(),
            }
            .store_in(out),
            Union::Seed(seed) => Word {
                status: OK,
                word: seed.to_bytes(),
            }
            .store_in(out),
        }
    }

    /// 收一条：**原样收下**（空帧、或长过这一族的缓冲 ⇒ `None`）。形状不在这里判——
    /// 见 [`Said`] 的照实记。
    fn fetch(bytes: &[u8]) -> Option<Said> {
        if bytes.is_empty() {
            return None;
        }
        let mut buf = [0u8; UNION_LEN];
        buf.get_mut(..bytes.len())?.copy_from_slice(bytes);
        Some(Said {
            buf,
            len: bytes.len(),
        })
    }
}

// ── 协调那一帧（**装配者 → 持树者**，不是门外那一问）──────────
//
// 它不在上面那张图里：上面那几帧是**客人 ↔ 持树者**的一问一答，这一条是**装配者递过来的
// 一格号**（装完那一位域之后一次）。两族同住本文件，因为"帧形只有一处"这一条不分装配期与
// 运行期——它是同一棵树的两半。

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
/// 一样在宿主上编得动；**"表外的眼睛码 ⇒ 整帧读不懂"那一条原先由 `judge` 靶钉着，那条判据
/// 随靶一并删了**（用户裁定"protocol-case 没必要"）——搬家的理由撤了一半，位置不动。
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
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CoordFrame {
    pub who: TaskId,
    pub eyes: Eyes,
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
