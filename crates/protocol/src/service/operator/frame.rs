//! operator 的**帧那一半** —— 帧、码、记号（内核那几只手的别名与适配在 `protocol` 那一侧的 `mod.rs`）。

use alloc::string::String;
use env::Mark;
use env::{PieToken, TaskId};

use super::path::{Path, PathBuf};
use crate::service::coalition::CoalitionId;
use crate::service::principal::PrincipalId;

use crate::id::Id as _;
use crate::message::Message;

/// 一枚条目的**号**：机器用的那一个。
/// **裸号**：与 [`PrincipalId`](crate::service::principal::PrincipalId) / [`CoalitionId`](crate::service::coalition::CoalitionId)
/// 同形（8 字节小端上线），不同源。线上解码面造得出任何号（[`EntryId::new`]），
/// "这枚号还在不在"由每条读**查一次表**答出来。
/// **没有 `ROOT`**（对照另两种号：那两处的 `ROOT` 都在，这里特意没有）：根不是谁条目里的
/// 一条，故**根没有号**——`EntryId(0)` 是第一个**真格子**（`sys`），不是"没有"。
/// "没有这个号"由 [`Fail::Unknown`] 答，别拿 0 当空。根要当坐标时走 [`Where::Root`]。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntryId(usize);

impl EntryId {
    /// 由裸号造一个（线上解码面；已失效的号从这里进来）。
    pub const fn new(raw: usize) -> EntryId {
        EntryId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

impl crate::id::Id for EntryId {
    fn new(raw: usize) -> EntryId {
        EntryId::new(raw)
    }

    fn get(self) -> usize {
        EntryId::get(self)
    }
}

/// **号那一格线上是 8 字节小端**——与 [`crate::id::Id`] 给三条号空间定的同一条规则（那一条 trait
impl env::wire::Field for EntryId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(Self::from_bytes(bytes.get(..8)?.try_into().ok()?))
    }
}

/// 一块 `Pane` 里最多几条。条数是策略，容器要有界。
pub const PANE_CAP: usize = 16;

/// **容器坐标**：要动的那一块 `Pane` 在哪。
/// 两种报法：**根**，或**某一号**。根必须显式占一格——**根没有号**（见 [`EntryId`]），
/// 所以它既不是"0 号"，也不能拿 `Option` 的空位代替：那两样都会被读成"某个真格子"。
/// 它的对立面是 [`Operator::find`] / [`Operator::trim`] / [`Operator::name`] 的形参：
/// 那三条要的是**条目**的号，**根根本递不进来**——这是类型义务，不是运行期检查。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// 根那一层：[`Operator::list`] 列的就是它，`land` / `part` 在它下面立一格。
    Root,
    /// 某一号那一块 `Pane` 里。
    At(EntryId),
}

/// 容器坐标那一格的"记"：`0` = 根、`1` = 号（[`Where`] 两种报法在线上的样子）。
const AT_ROOT: u8 = 0;
const AT_ID: u8 = 1;

/// **容器坐标那一格是"记 ＋ 号"**（9 字节）：`0` = 根（后面 8 字节**照写零**）、`1` = 某一号。
impl env::wire::Field for Where {
    const WIDTH: usize = 1 + <EntryId as env::wire::Field>::WIDTH;

    fn store(&self, out: &mut [u8]) {
        let (tag, id) = match *self {
            Where::Root => (AT_ROOT, EntryId::new(0)),
            Where::At(id) => (AT_ID, id),
        };
        out[0] = tag;
        // 长度恰是 `WIDTH`（`Field::store` 的契约）⇒ 记之后那一段正好是号那一格。
        id.store(&mut out[1..]);
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            AT_ROOT => Some(Where::Root),
            AT_ID => Some(Where::At(EntryId::fetch(bytes.get(1..)?)?)),
            _ => None,
        }
    }
}

/// 八条原语会失败在哪一格。**一格对应一个不同的下一步**。
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
/// **后两格（[`Fail::Denied`] / [`Fail::Unjudged`]）来自门外那一问**：核心一个字节都不知道
/// 它们，但它们同样是**客侧要按下一步区分**的
/// 答案 ⇒ 与前面六格同住这一枚类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 那一号/那一格不在树上 ⇒ 换个名字重来，或者先把中间那一层分出来。
    Unknown,
    /// 那块 `Pane` 里还有东西，而这一手会**毁掉**里面的 ⇒ 先清空。
    /// 今天只有两条原语走得到它：[`Operator::land`] 的换绑（要把那块非空 `Pane` 换成砖）与
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者往它里面走。
    NotATile,
    /// 那一号不是一块 `Pane`（是一枚 `Tile`）⇒ 走不进去；列的时候则说明"那是枚 `Tile`，没什么可列"。
    NotAPane,
    /// 那一块 `Pane` 已经 [`PANE_CAP`] 条，装不下 ⇒ 拆层 / 扩容量。
    Full,
    Dead,
    Denied,
    /// 门外那一问答"判不了"：[`UNJUDGED`] ——要问的那条事实问不到。
    /// **它不承诺"等一会儿会好"**：对面不答 / 超时（会好），与那一号是碑 / 那一格是块窗格 /
    Unjudged,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permit {
    Unset,
    /// 就是这一位。
    Trunk(PrincipalId),
    /// 这一位在 `p` 那一支里（`p ≼ 本人`，含相等）——纵向那条轴。
    Bough(PrincipalId),
    /// 这一位在这枚盟里——横向那条轴。
    Among(CoalitionId),
    /// **就是开着第 `e` 格的那一位**（那一格的坐标是 [`EntryId`]，不是身份号）。
    Opener(EntryId),
}

/// 「用那一轴」在帧里的标记。**没有许可**那一档是 `0`。
const PERMIT_NONE: u8 = 0;
const PERMIT_TRUNK: u8 = 1;
const PERMIT_BOUGH: u8 = 2;
const PERMIT_AMONG: u8 = 3;
/// `4` 之后的号装的是**格号**（[`Permit::Opener`]），不是身份号——同一个 8 字节那一格。
const PERMIT_OPENER: u8 = 4;

/// 「**用**」那一轴在线上是"**标记 ＋ 8 字节号**"（9 字节）。
/// 口径与 [`EntryId`] 那一处相同：**impl 跟着类型走**——这是 [`Permit`] 自己的编码，
impl env::wire::Field for Permit {
    const WIDTH: usize = 1 + 8;

    fn store(&self, out: &mut [u8]) {
        let (tag, id) = match *self {
            Permit::Unset => (PERMIT_NONE, 0),
            Permit::Trunk(p) => (PERMIT_TRUNK, p.get() as u64),
            Permit::Bough(p) => (PERMIT_BOUGH, p.get() as u64),
            Permit::Among(c) => (PERMIT_AMONG, c.get() as u64),
            // 格号与身份号同宽（都是 8 字节）⇒ 帧长一个字节都不动。
            Permit::Opener(e) => (PERMIT_OPENER, e.get() as u64),
        };
        out[0] = tag;
        out[1..].copy_from_slice(&id.to_le_bytes());
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let tag = *bytes.first()?;
        let raw: [u8; 8] = bytes.get(1..9)?.try_into().ok()?;
        let id = u64::from_le_bytes(raw);
        Some(match tag {
            PERMIT_NONE => Permit::Unset,
            PERMIT_TRUNK => Permit::Trunk(PrincipalId::new(id as usize)),
            PERMIT_BOUGH => Permit::Bough(PrincipalId::new(id as usize)),
            PERMIT_AMONG => Permit::Among(CoalitionId::new(id as usize)),
            PERMIT_OPENER => Permit::Opener(EntryId::new(id as usize)),
            _ => return None,
        })
    }
}

/// **门外那一问的答案**。三格；`Allow` / `Deny` 各一个不同的下一步，`Unjudged` 是"判不了"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ruling {
    /// 过。
    Allow,
    /// 不过——**终态**：换人 / 换目标 / 别重试。
    Deny,
    /// **判不了**：这一问要的那条事实问不到——对面不答 / 超时（**会好**），或那一号是碑 /
    /// 那一格是块窗格 / 开者那扇门封印了（**好不了**）。
    /// 两类在这里**同格**：客人的下一步是同一个（当趟放弃），差别在"为什么"⇒ 那是读数
    /// （[`Facts::opens`] 那一侧的三因分得开）。**重试是客人的策略**，本格不作承诺。
    Unjudged,
}

// 七个动作在报文里的码——**与核心那七条原语同名**（`land` / `part` / `find` / `trim` /
// `list` / `seek` / `name`）：线上与模型是同一件事的两层，不该各起一套词。
// **它们不再是协议面**（照板那一族的先例）：编的那一侧由 [`Req`] 说、解的那一侧由 [`Wire`]
// 说，每一枚码各被读一次（字段表头一格 `op`）。外面认的是类型 ⇒ 降为私有——没有读者的格不
// 留在面上。
const LAND: u8 = 1;
const PART: u8 = 2;
const FIND: u8 = 3;
const TRIM: u8 = 4;
const LIST: u8 = 5;
const NAME: u8 = 6;
const SEEK: u8 = 7;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格。**前六格与 [`Fail`] 的前六格一一对应**，第七格不是失败域的：这一问读不懂
/// （帧坏了 ⇒ 不猜、不崩）。**第八、九格来自门外那一问**（判据那一半住
/// `programs/src/system/operator/core/judge.rs`），它们与 [`Fail::Denied`] / [`Fail::Unjudged`]
/// 一一对应。
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持树者那一侧编、客人那一侧读）。
pub const UNKNOWN: u8 = 1;
pub const NONEMPTY: u8 = 2;
pub const NOTATILE: u8 = 3;
pub const NOTAPANE: u8 = 4;
pub const FULL: u8 = 5;
pub const DEAD: u8 = 6;
pub const BAD: u8 = 7;
pub const DENIED: u8 = 8;
/// **门外那一问答"判不了"**：这一问要的那条事实问不到——对面不答 / 超时（**会好**），
/// 或那一号是碑 / 那一格是块窗格 / 开者那扇门封印了（**好不了**）。
/// 与 [`DENIED`] 分家的理由只有一条，但够硬：**"没资格"与"判不了"是两件事**——混成一格，
/// 就会把"身份服务挂了"读成"我没权限"，整机去查规矩。**它不承诺"等一会儿会好"**：
/// 两类因在客人那一侧是同一个下一步（当趟放弃），把三因分开的是**读数**，不是第三格码。
pub const UNJUDGED: u8 = 9;

/// 问话那一侧的上界：**最长那一条**（`Road`：`op` ＋ [`Path::LEN`]）。
/// 服务端按它备一只缓冲（收下来的帧不会超过它），各条问话的**实际**长度由形状说——定长那几条
/// 是字段表求和（`LEN`），`Road` 那一格是 [`RoadFrame::store_at`] 交回的游标。
pub const REQ_LEN: usize = RoadFrame::LEN;

/// 一答的**上限**：四种答形里最大的那一形（`[status][条数][号…]`）。一条 `Pane` 本来就不超过
/// [`PANE_CAP`] 枚 ⇒ **一趟答得完，没有"未完"那一格**（对照 `coalition` 那一侧：盟籍
/// 没有上限，故那里必须带一格"未完"）。
/// 本族那只缓冲就是它（[`Message::Buf`]）；另两形都短于它——编译期钉住（`名` 那一形最长是
/// 状态 ＋ 名字那一格的上界（31 字节），`号` 那一形是状态 ＋ 8）。
pub const UNION_LEN: usize = 2 + PANE_CAP * 8;

// 31 = 名字那一格在**这一族**里的上界（长度那一字节不在这一形里：长度即内容）。
const _: () = assert!(Status::LEN + 31 <= UNION_LEN);
const _: () = assert!(Status::LEN + <[u8; 8] as env::wire::Field>::WIDTH <= UNION_LEN);

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct RoadFrame {
    pub op: u8,
    pub road: PathBuf,
}

const _: () = assert!(RoadFrame::LEN == 1 + Path::LEN);

/// `List` 那一问：动作码 ＋ 容器坐标。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct List {
    pub op: u8,
    pub at: Where,
}

/// `Part` 那一问：动作码 ＋ 容器坐标 ＋ 新名。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 42)]
pub struct Part {
    pub op: u8,
    pub at: Where,
    pub name: String,
}

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 60)]
pub struct Land {
    pub op: u8,
    pub at: Where,
    pub name: String,
    pub entry: PieToken,
    pub mine: bool,
    pub permit: Permit,
}

/// `Find` / `Trim` / `Name` 那三问**共用**的形状：动作码 ＋ 一枚号。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub op: u8,
    pub id: EntryId,
}

/// **一问的荷载**——一个动作一条形状，没有"报法"那一格可以填错。
/// 号那一侧全按 [`EntryId`] 走；名字只出现在两条路上：[`Req::Road`]（`seek` 收的那条路）
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Req {
    Road(PathBuf),
    /// `list`：列那一块 `Pane` 里的号。
    List(Where),
    /// `part`：在那一块 `Pane` 下，给这个新名分一格。
    Part {
        at: Where,
        name: String,
    },
    /// `land`：在那一块 `Pane` 下，给这个新名落一枚。
    /// `entry` 是**经会话交出去之后**、种在持树者表里的那一个号（`ship` 换回来的），
    /// 不是"客人的 Pie 是几号"——两个编号空间不同源。
    Land {
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: bool,
    },
    /// `find`：那一号后面那一枚 Pie。
    Find(EntryId),
    /// `trim`：把那一号剪掉。
    Trim(EntryId),
    /// `name`：那一号此刻叫什么。
    Name(EntryId),
}

/// **解开的一问**（名字已经是 [`String`]，故不是借用）。
/// 与 [`Req`] 是一对：编的时候按动作分形状，解的时候也按动作分形状——`op` 与荷载不配
/// （比如 `LAND` 那一码配上一枚号）解不出来，持树者据此答 [`BAD`]。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wire {
    /// `seek`：一条路（段数与段都在 [`Path`] 里；超上限根本造不出来，故 [`FULL`] 不再来自它）。
    Road(PathBuf),
    /// `list`：容器坐标。
    List(Where),
    /// `part`：容器坐标 + 新名。
    Part {
        at: Where,
        name: String,
    },
    Land {
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: bool,
    },
    /// `find` / `trim` / `name`：一枚号（三者的形状一样，故解出来仍是三格）。
    Find(EntryId),
    Trim(EntryId),
    Name(EntryId),
}

impl Message for Req {
    type In = Wire;
    /// 这一族的缓冲：**最长那一条**（[`REQ_LEN`]）。
    type Buf = [u8; REQ_LEN];
    const EMPTY: Self::Buf = [0u8; REQ_LEN];

    /// 编进 `out`：**动作码由形状给**（不在别处再写一遍），偏移与长度由字段表求和。
    /// `Road` 那一格的正文（路）也回表了（[`RoadFrame`]：动作码 ＋ 路）——偏移一处都不写。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Req::Road(road) => RoadFrame {
                op: SEEK,
                road: road.clone(),
            }
            .store_at(out, 0),
            Req::List(at) => List { op: LIST, at: *at }.store_at(out, 0),
            Req::Part { at, name } => Part {
                op: PART,
                at: *at,
                name: name.clone(),
            }
            .store_at(out, 0),
            Req::Land {
                at,
                name,
                entry,
                permit,
                mine,
            } => Land {
                op: LAND,
                at: *at,
                name: name.clone(),
                entry: *entry,
                mine: *mine,
                permit: *permit,
            }
            .store_at(out, 0),
            Req::Find(id) => Entry { op: FIND, id: *id }.store_at(out, 0),
            Req::Trim(id) => Entry { op: TRIM, id: *id }.store_at(out, 0),
            Req::Name(id) => Entry { op: NAME, id: *id }.store_at(out, 0),
        }
    }

    /// 解开一问：**`op` 决定形状**（见文件头那张表）。**读不懂返 `None`**（持树者据此答
    /// [`BAD`]）。
    /// **长度为该形状该有的长度是帧的契约**（各张表的 `LEN`，`store` 产出的就是那个长度），
    /// 故短一字节、长一字节都读不懂。
    fn fetch(bytes: &[u8]) -> Option<Wire> {
        let op = *bytes.first()?;
        Some(match op {
            // **长度即形状**：`1 ＋ 1 ＋ 段数 × 32`（段数那一格在 [`Path`] 里；条数与长度对不对
            // 由下面那一句判——短一字节、长一字节都答"读不懂"）。
            SEEK => {
                let (frame, at) = RoadFrame::fetch_at(bytes, 0)?;
                if at != bytes.len() {
                    return None;
                }
                Wire::Road(frame.road)
            }
            LIST if bytes.len() == List::LEN => Wire::List(List::fetch(bytes)?.at),
            // 这两形含一枚变长名字 ⇒ **"恰好"按游标判**（帧长不再等于那张表的 `LEN`）。
            PART => {
                let (frame, end) = Part::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Part {
                    at: frame.at,
                    name: frame.name,
                }
            }
            LAND => {
                let (frame, end) = Land::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Land {
                    at: frame.at,
                    name: frame.name,
                    entry: frame.entry,
                    permit: frame.permit,
                    mine: frame.mine,
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

/// 一帧「列」的读数：号最多 [`PANE_CAP`] 枚。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    ids: [EntryId; PANE_CAP],
    n: usize,
}

impl Listing {
    /// 空的那一串。
    pub const fn new() -> Listing {
        Listing {
            ids: [EntryId::new(0); PANE_CAP],
            n: 0,
        }
    }

    /// 收一串（**收够 [`PANE_CAP`] 枚就停**：一条 pane 本来就不超过它）。
    pub fn of(ids: impl Iterator<Item = EntryId>) -> Listing {
        let mut listing = Listing::new();
        for id in ids.take(PANE_CAP) {
            listing.push(id);
        }
        listing
    }

    /// 按号序（就是帧里的次序）走一遍。
    pub fn iter(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.ids[..self.n].iter().copied()
    }

    /// 那一段号——**编那一侧要它**（`store_tail` 走的是一条切片，不是一个迭代器）。
    pub fn as_slice(&self) -> &[EntryId] {
        &self.ids[..self.n]
    }

    /// 收一枚。**满了就丢**：一条 pane 本来就不超过 [`PANE_CAP`] 枚。
    fn push(&mut self, id: EntryId) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = id;
            self.n += 1;
        }
    }
}

/// **头一格**：状态。它自己就是"一格状态"那一形（六格失败与"门外那两格"都走它），也是另外
/// 三形的起头。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「列」那一形的**头两格**：状态 ＋ **条数**（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tally {
    pub status: u8,
    pub count: u8,
}

/// 「号」那一形：`[status][8 字节]`——**定长 9**（`part` / `seek` 答坐标、`find` 答门闩，
/// 线上逐字同形）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Word {
    pub status: u8,
    pub word: [u8; 8],
}

/// **一答的形状**——答有四种：一格状态 / 一串号 / 一枚名字 / 一枚号。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Union {
    /// 一格状态（成功 / 六格失败 / 门外那两格）——**没有任何荷载**。
    Status(u8),
    /// `list` 的下场：一串号。
    List(Listing),
    /// `name` 的下场：一枚名字（**长度即名长**）。
    Name(String),
    /// `part` / `seek` 的下场：那一格**坐标**。
    Entry(EntryId),
    /// `find` 的下场：那一格是"我给你的那一枚**在你表里**是几号"（[`PieToken`]）。
    /// **与 [`Union::Entry`] 同形不同物**（都是 `[OK][8 字节]`）而**另起一格、不复用**：两枚号
    /// 类型不同，混用就是把"树的坐标"与"你表里的门闩"当成一件事。
    Seed(PieToken),
}

/// **收进来的一答**：**原样的字节** ＋ 四个读法。
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
    pub fn code(&self) -> u8 {
        self.bytes().first().copied().unwrap_or(BAD)
    }

    /// 按「号」那一形读（`land` / `part` / `seek` 的下场）：`[status][8 字节]` → **坐标**。
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
    /// 名字读不懂（空 / 太长 / 含 NUL / 不是 UTF-8）⇒ `Err(BAD)`：那一侧旧日的四格失败域
    /// 在这里**归一格**——问的人能做的补救是同一件（这一帧坏了，重问）。
    pub fn name(&self) -> Result<String, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let text = env::wire::fetch_bytes(bytes, Status::LEN).ok_or(BAD)?;
        Ok(String::from(core::str::from_utf8(text).map_err(|_| BAD)?))
    }

    /// 按「列」那一形读（`list` 的下场）：`[status][条数][号…]` → 一串号。
    /// **帧长即条数**：条数与剩下那些字节对不上（或条数超过 [`PANE_CAP`]）⇒
    /// `Err(BAD)`——短一字节也是它。
    pub fn list(&self) -> Result<Listing, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let head = Tally::fetch(bytes).ok_or(BAD)?;
        let count = head.count as usize;
        if count > PANE_CAP {
            return Err(BAD);
        }
        let body = bytes.get(Tally::LEN..).ok_or(BAD)?;
        let mut ids = [EntryId::new(0); PANE_CAP];
        let end = env::wire::fetch_tail(body, 0, &mut ids[..count]).ok_or(BAD)?;
        if end != body.len() {
            return Err(BAD);
        }
        Ok(Listing::of(ids[..count].iter().copied()))
    }
}

impl Message for Union {
    type In = Said;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    /// 编进 `out`：状态由形状给（不在别处再写一遍），变长那两段交给 `env::wire` 的两个尾巴。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Union::Status(code) => Status { status: *code }.store_at(out, 0),
            Union::List(list) => {
                let ids = list.as_slice();
                let head = Tally {
                    status: OK,
                    count: ids.len() as u8,
                };
                head.store_at(out, 0)?;
                env::wire::store_tail(out, Tally::LEN, ids)
            }
            Union::Name(name) => {
                let at = Status { status: OK }.store_at(out, 0)?;
                // 名长即这一帧剩下的那些字节（**长度即内容**那一形）。
                env::wire::store_bytes(out, at, name.as_bytes())
            }
            Union::Entry(id) => Word {
                status: OK,
                word: id.to_bytes(),
            }
            .store_at(out, 0),
            Union::Seed(seed) => Word {
                status: OK,
                word: seed.to_bytes(),
            }
            .store_at(out, 0),
        }
    }

    /// 收一条：**原样收下**（空帧、或长过这一族的缓冲 ⇒ `None`）。形状不在这里判——
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

// 它不在上面那张图里：上面那几帧是**客人 ↔ 持树者**的一问一答，这几条是**装配者递过来
// 的东西**（立一条路 / 一位客人）。两族同住本文件，因为"帧形只有一处"这一条不分装配期与
// 运行期——它是同一棵树的两半。两形的总说明与 `Tip` / `TipIn` 在下面。（"一格号"那一形

// 两形走**同一个洞、同一个读者**（提示之路 = 装配侧 → 持树者）：既不经过会话、也没有客人
// ——"往树上立一路"由持树者在自己核里做（`programs/src/system/operator/plate.rs::plate`）。
// **首格 `kind` 说这一帧是哪一形**——与客人那一族的动作码同一条纪律：一个动作一条形状，

/// 提示之路上的两个 `kind`（首格；表外 ⇒ 这一帧读不懂）。
const TIP_PLATE: u8 = 1;
const TIP_GUEST: u8 = 2;
const TIP_WIRED: u8 = 3;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GuestFrame {
    pub kind: u8,
    pub who: TaskId,
}

/// 「门禁接线」那一形：**只有一个字节**（`kind`）——一句话，不带号。
/// **它说的是什么**：装配者已经把**名册**认下来了（补绑它自己与树），从那以后持树者那道门
/// **问得动身份**（`operator::door::may`）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WiredFrame {
    pub kind: u8,
}

/// **这一趟落格要带的那句规矩**（"立一条路"那一形上的第二轴）。
/// # 为什么只有两格，且没有"填一枚 `Permit`"这一路
/// 这条路上装的是**装配者**（它请持树者替它落格）。装配者**报不出任何号**——它没有名录面
/// （`Roster` 只有 `bind` / `adopt`），也没有读格的那几手（`Tree` 只有"递上去"）⇒ 一枚
/// `Permit` 里的号它一个都填不了。故这一轴说的是**要不要带规矩**，而不是"带哪一条"；
/// 而"带哪一条"由 [`Rule::Root`] 自己钉死——那是这一族今天**说得出口**的唯一一句真话
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule {
    /// **不记许可**：与 `/svc/sys/operator/{…}` 那七格同一条口径——任何已绑身份都取得回。
    None,
    Root,
}

impl Rule {
    pub const WIDTH: usize = 1;
}

impl env::wire::Field for Rule {
    const WIDTH: usize = Rule::WIDTH;

    fn store(&self, out: &mut [u8]) {
        out[0] = match *self {
            Rule::None => 0,
            Rule::Root => 1,
        };
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            0 => Some(Rule::None),
            1 => Some(Rule::Root),
            // 表外的记 ⇒ 整帧读不懂（同 `Permit` / `Where` 那一格的口径）。
            _ => None,
        }
    }
}

/// `Plate` 那一句：首格 `kind` ＋ 一条路 ＋ 末段那一枚 ＋ 规矩一格。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct PlateFrame {
    pub kind: u8,
    pub road: PathBuf,
    pub leaf: PieToken,
    pub rule: Rule,
}

const _: () = assert!(PlateFrame::LEN == 1 + Path::LEN + PieToken::WIDTH + Rule::WIDTH);

/// 提示之路上**最长那一形**的宽度（立一条路：[`PlateFrame`]）——两侧各备一只这么大的缓冲，
/// 收的那一侧按它拉。
pub const TIP_LEN: usize = PlateFrame::LEN;

/// **装配者推给持树者的一句话**（提示之路那一帧）。
/// 三形，各自的正文在变体上；共用的两句话：
/// - **树不能当自己的客人**：把一格挂上树在别处都是**客人**那一趟（`part` ＋ `land` 两问走一条
///   会话），而树没有那条会话——它的生我者（编排域）是**替每一位客人转授**的那一侧，替不了
///   自己（自指 ⇒ 环）。树手里本来就握着**核**（`Operator::land` / `part`）
///   ⇒ "装配者递东西、持树者自己落"。
/// - **名字随帧来**：持树者不认识任何一族的名字（`control::frame::DIR` / `NAME` 都是递帧那一侧
///   的事实），它只答"把这一条路立出来"。
pub enum Tip {
    /// **在树上立一条路**：前缀逐段立成窗格（缺的就地造），末段按 `leaf` 落叶子或立窗格。
    /// 路是**绝对坐标**（从根起数），故 `/svc/sys/control`、`/svc/sys/operator`、`/svc/sys/operator/part`
    /// 三种落法**同一个形状**说得出来；再深一层、或"父底下立一块窗格"也不需要新格
    Plate {
        road: PathBuf,
        leaf: PieToken,
        rule: Rule,
    },
    /// **这一位是客人**。
    Guest(TaskId),
    /// **门禁接线**（装配者已认下名册）：一句话，不带号。
    Wired,
}

impl Tip {
    /// 编进 `out`，返写完的游标；装不下 / **路空** ⇒ `None`（路本身合法由 [`Path`] 保证）。
    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Tip::Plate { road, leaf, rule } => {
                if road.is_empty() {
                    return None;
                }
                PlateFrame {
                    kind: TIP_PLATE,
                    road: road.clone(),
                    leaf: *leaf,
                    rule: *rule,
                }
                .store_at(out, 0)
            }
            Tip::Guest(who) => GuestFrame {
                kind: TIP_GUEST,
                who: *who,
            }
            .store_at(out, 0),
            Tip::Wired => WiredFrame { kind: TIP_WIRED }.store_at(out, 0),
        }
    }
}

/// **解开的一句**：路已经收进自己那一份（[`Path`]）。
pub enum TipIn {
    /// 立一条路（前缀逐段立窗格，末段按 `leaf`），并按 `rule` 决定要不要带一句规矩。
    Plate {
        road: PathBuf,
        leaf: PieToken,
        rule: Rule,
    },
    /// 这一位是客人。
    Guest(TaskId),
    /// 门禁接线。
    Wired,
}

impl TipIn {
    /// 解开一句：**首格 `kind` 决定形状**，长度必须是那一形该有的长度。
    /// 读不懂（表外的 `kind` / 路空 / 段数越界 / 长度不对）⇒ `None`：持树者据此报一行读数
    /// ——这条路上没有答话那一格，**别静默丢**。
    pub fn fetch(bytes: &[u8]) -> Option<TipIn> {
        match *bytes.first()? {
            TIP_PLATE => {
                let (frame, at) = PlateFrame::fetch_at(bytes, 0)?;
                // **路空**是这一族的规矩（"末段"必须有）；**长度**也是形状的一部分
                // （`1 ＋ 1 ＋ 段数 × 32 ＋ 8 ＋ 1`：长短都不认）。两句都是本族的，derive 不替它判。
                if frame.road.is_empty() || at != bytes.len() {
                    return None;
                }
                Some(TipIn::Plate {
                    road: frame.road,
                    leaf: frame.leaf,
                    rule: frame.rule,
                })
            }
            TIP_GUEST if bytes.len() == GuestFrame::LEN => {
                Some(TipIn::Guest(GuestFrame::fetch(bytes)?.who))
            }
            TIP_WIRED if bytes.len() == WiredFrame::LEN => Some(TipIn::Wired),
            _ => None,
        }
    }
}

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    /// **九格成一枚完整双射**：前六格是核心自己的失败，后两格是门外那一问的两格
    /// ——`DENIED` / `UNJUDGED` 本来就在线上答得出来，故客侧读得回来。`BAD` 在表外。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::NonEmpty => NONEMPTY,
    Fail::NotATile => NOTATILE,
    Fail::NotAPane => NOTAPANE,
    Fail::Full => FULL,
    Fail::Dead => DEAD,
    Fail::Denied => DENIED,
    Fail::Unjudged => UNJUDGED,
}

// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）。
/// **带面名**（本族那一枚是 `operator-ask`，提示那一枚是 `*-tip`）：问话孔的认领键是
/// "**谁开的 + 记号**"，而**同一枚任务可能同时是两族的客人**（`canonical` / `guest` / `principal`
/// …都是）——两枚孔都铸在**它自己那张表**里，记号再一样就分不开了。
pub const ASK_MARK: Mark = Mark::of("operator-ask");

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）。
pub const TIP_MARK: Mark = Mark::of("tip");

const _: () = assert!(ASK_MARK.get() != Mark::of("ask").get());
const _: () = assert!(ASK_MARK.get() != TIP_MARK.get());
