//! operator::frame 的**词汇那一半**：号（`EntryId`）· 位置（`Where`）· 失败词汇（`Fail`）·
//! 许可（`Permit`）· 判定（`Ruling`）· 动作码与状态码 · 记号（`LINK`/`ASK_MARK`/`TIP_MARK`）。

use env::Mark;

use crate::service::coalition::CoalitionId;
use crate::service::principal::PrincipalId;
use crate::wire::OK;   // `WireCodes` 派生的两向读法要用它（本文件是枚举的家）

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

/// 八条原语会失败在哪一格。**一格对应一个不同的下一步**。
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （见 [`Operator::land`] / [`Operator::part`]）；而 owner 归 Principal，Operator 分不出
/// "自己 / 别人"，所以"已占即拒"在这里无处落脚。
/// **后两格（[`Fail::Denied`] / [`Fail::Unjudged`]）来自门外那一问**：核心一个字节都不知道
/// 它们，但它们同样是**客侧要按下一步区分**的
/// 答案 ⇒ 与前面六格同住这一枚类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::WireCodes)]
#[wire(also(BAD = 7))]
pub enum Fail {
    /// 那一号/那一格不在树上 ⇒ 换个名字重来，或者先把中间那一层分出来。
    #[code(1)]
    Unknown,
    /// 那块 `Pane` 里还有东西，而这一手会**毁掉**里面的 ⇒ 先清空。
    /// 今天只有两条原语走得到它：[`Operator::land`] 的换绑（要把那块非空 `Pane` 换成砖）与
    #[code(2)]
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者往它里面走。
    #[code(3)]
    NotATile,
    /// 那一号不是一块 `Pane`（是一枚 `Tile`）⇒ 走不进去；列的时候则说明"那是枚 `Tile`，没什么可列"。
    #[code(4)]
    NotAPane,
    /// 那一块 `Pane` 已经 [`PANE_CAP`] 条，装不下 ⇒ 拆层 / 扩容量。
    #[code(5)]
    Full,
    #[code(6)]
    Dead,
    #[code(8)]
    Denied,
    /// 门外那一问答"判不了"：[`UNJUDGED`] ——要问的那条事实问不到。
    /// **它不承诺"等一会儿会好"**：对面不答 / 超时（会好），与那一号是碑 / 那一格是块窗格 /
    #[code(9)]
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

pub const LAND: u8 = 1;

pub const PART: u8 = 2;

pub const FIND: u8 = 3;

pub const TRIM: u8 = 4;

pub const LIST: u8 = 5;

pub const NAME: u8 = 6;

pub const SEEK: u8 = 7;

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

/// `4` 之后的号装的是**格号**（[`Permit::Opener`]），不是身份号——同一个 8 字节那一格。
/// 「用那一轴」在帧里的标记。**没有许可**那一档是 `0`。
/// 容器坐标那一格的"记"：`0` = 根、`1` = 号（[`Where`] 两种报法在线上的样子）。
const AT_ROOT: u8 = 0;

const AT_ID: u8 = 1;

const PERMIT_NONE: u8 = 0;

const PERMIT_TRUNK: u8 = 1;

const PERMIT_BOUGH: u8 = 2;

const PERMIT_AMONG: u8 = 3;

const PERMIT_OPENER: u8 = 4;

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
