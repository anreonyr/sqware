//! :frame 的词汇那一半：号（EntryId）· 位置（Where）· 失败词汇（Fail）·
//! 许可（`Permit`）· 判定（`Ruling`）· 动作码与状态码 · 记号（`LINK`/`ASK_MARK`/`TIP_MARK`）。

use env::Mark;

use crate::service::identity::Selector;
use crate::wire::OK; // `WireCodes` 派生的两向读法要用它（本文件是枚举的家）

/// 一枚条目的**号**：机器用的那一个
/// **裸号**：与 PrincipalId / CoalitionId
/// 同形（8 字节小端上线），不同源。线上解码面造得出任何号（EntryId::new）
/// "这枚号还在不在"由每条读**查一次表**答出来
/// 一条，故**根没有号**——`EntryId(0)` 是第一个**真格子**（`sys`），不是"没有"
/// "没有这个号"由 Fail::Unknown 答，别拿 0 当空。根要当坐标时走 Where::Root
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EntryId(usize);

impl EntryId {
    pub const fn new(raw: usize) -> EntryId {
        EntryId(raw)
    }

    /// 裸号
    pub const fn get(self) -> usize {
        self.0
    }
}

/// 一块 Pane 的有界容量；必须容纳统一 Identity 的 17 个独立动作面。
/// 核心和 Listing 帧共用此界，不允许服务已落下而列表静默截断。
pub const PANE_CAP: usize = 32;

/// **容器坐标**：要动的那一块 `Pane` 在哪
/// 两种报法：**根**，或**某一号**。根必须显式占一格——**根没有号**（见 EntryId）
/// 所以它既不是"0 号"，也不能拿 `Option` 的空位代替：那两样都会被读成"某个真格子"
/// 它的对立面是 Operator::find / Operator::trim / Operator::name 的形参
/// 那三条要的是**条目**的号，**根根本递不进来**——这是类型义务，不是运行期检查
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// 根那一层：Operator::list 列的就是它，`land` / `part` 在它下面立一格
    Root,
    /// 某一号那一块 `Pane` 里
    At(EntryId),
}

/// 八条原语会失败在哪一格。**一格对应一个不同的下一步**
/// **没有"名字已被占"那一格**：同名接手一枚 `Tile`、或一块**空的** `Pane`，都是换绑
/// （ / Operator::part）；而 owner 归 Principal，Operator 分不出
/// **后两格（Fail::Denied / Fail::Unjudged）来自门外那一问**：核心一个字节都不知道
/// 它们，但它们同样是**客侧要按下一步区分**的
/// 答案 ⇒ 与前面六格同住这一枚类型
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::WireCodes)]
#[wire(also(BAD = 7))]
pub enum Fail {
    /// 那一号/那一格不在树上 ⇒ 换个名字重来，或者先把中间那一层分出来
    #[code(1)]
    Unknown,
    #[code(2)]
    NonEmpty,
    /// 寻到头是一块 `Pane`，不是一枚 `Tile` ⇒ 改用列，或者往它里面走
    #[code(3)]
    NotATile,
    /// 那一号不是一块 `Pane`（是一枚 `Tile`）⇒ 走不进去；列的时候则说明"那是枚 `Tile`，没什么可列"
    #[code(4)]
    NotAPane,
    /// 那一块 `Pane` 已经 PANE_CAP 条，装不下 ⇒ 拆层 / 扩容量
    #[code(5)]
    Full,
    #[code(6)]
    Dead,
    #[code(8)]
    Denied,
    /// 门外那一问答"判不了"：UNJUDGED ——要问的那条事实问不到
    /// **它不承诺"等一会儿会好"**：对面不答 / 超时（会好），与那一号是碑 / 那一格是块窗格 /
    #[code(9)]
    Unjudged,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permit {
    /// 不问身份；身份服务离线也能使用。
    Public,
    /// 请求者必须有绑定。
    Bound,
    /// 一次由身份权威判断当前有效身份。
    Identity(Selector),
    /// **就是开着第 `e` 格的那一位**（那一格的坐标是 EntryId，不是身份号）
    Opener(EntryId),
}

/// **门外那一问的答案**。三格；`Allow` / `Deny` 各一个不同的下一步，`Unjudged` 是"判不了"
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ruling {
    /// 过
    Allow,
    /// 不过——**终态**：换人 / 换目标 / 别重试
    Deny,
    /// **判不了**：这一问要的那条事实问不到——对面不答 / 超时（**会好**），或那一号是碑 /
    /// 那一格是块窗格 / 开者那扇门封印了（**好不了**）
    /// （Facts::opens 那一侧的三因分得开）。**重试是客人的策略**，本格不作承诺
    Unjudged,
}

// 每一位动作在报文里的码——**与核心那几条原语同名**（下面那几枚常量就叫那个名字）：
// 线上与模型是同一件事的两层，不该各起一套词。
// **它们不再是协议面**（照板那一族的先例）：编的那一侧由 Req 说、解的那一侧由 Wire
// 说，每一枚码各被读一次（字段表头一格 `op`）。外面认的是类型 ⇒ 降为私有——没有读者的格不
// 留在面上。

pub const LAND: u8 = 1;

pub const PART: u8 = 2;

pub const FIND: u8 = 3;

pub const TRIM: u8 = 4;

pub const LIST: u8 = 5;

pub const NAME: u8 = 6;

pub const SEEK: u8 = 7;

/// `watch` 那一问：订一条子树（路 ＋ 订阅者那一页 ＋ 那一枚铃）
pub const WATCH: u8 = 8;

/// 树那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）
pub const LINK: &str = "operator";

/// 问话孔那一枚上的记号（两侧同一个：客人铸它时刻上去的，持树者按它认领那枚孔）
/// **带面名**（本族那一枚是 `operator-ask`，提示那一枚是 `*-tip`）：问话孔的认领键是
/// "**谁开的 + 记号**"，而**同一枚任务可能同时是两族的客人**（`canonical` / `guest` / `principal`
/// …都是）——两枚孔都铸在**它自己那张表**里，记号再一样就分不开了
pub const ASK_MARK: Mark = Mark::of("operator-ask");

/// 提示孔那一枚上的记号（持树者铸它时刻上去的；装配者按它认领那一枚）
pub const TIP_MARK: Mark = Mark::of("tip");

const _: () = assert!(ASK_MARK.get() != Mark::of("ask").get());

const _: () = assert!(ASK_MARK.get() != TIP_MARK.get());

/// 容器坐标那一格的"记"：`0` = 根、`1` = 号（Where 两种报法在线上的样子）
const AT_ROOT: u8 = 0;

const AT_ID: u8 = 1;

const PERMIT_PUBLIC: u8 = 0;
const PERMIT_BOUND: u8 = 1;
const PERMIT_IDENTITY: u8 = 2;
const PERMIT_OPENER: u8 = 3;

/// **容器坐标那一格是"记 ＋ 号"**（9 字节）：`0` = 根（后面 8 字节**照写零**）、`1` = 某一号
impl env::wire::Field for Where {
    const WIDTH: usize = 1 + <EntryId as env::wire::Field>::WIDTH;

    fn store(&self, out: &mut [u8]) {
        let (tag, id) = match *self {
            Where::Root => (AT_ROOT, EntryId::new(0)),
            Where::At(id) => (AT_ID, id),
        };
        out[0] = tag;
        // 长度恰是 `WIDTH`（Field::store 的契约）⇒ 记之后那一段正好是号那一格。
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
/// 标记与完整 Selector（包括 authority）。不用的荷载必须为零。
impl env::wire::Field for Permit {
    const WIDTH: usize = 1 + <Selector as env::wire::Field>::WIDTH;

    fn store(&self, out: &mut [u8]) {
        out.fill(0);
        match *self {
            Permit::Public => out[0] = PERMIT_PUBLIC,
            Permit::Bound => out[0] = PERMIT_BOUND,
            Permit::Identity(selector) => {
                out[0] = PERMIT_IDENTITY;
                selector.store(&mut out[1..]);
            }
            Permit::Opener(id) => {
                out[0] = PERMIT_OPENER;
                id.store(&mut out[1..9]);
            }
        }
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..Self::WIDTH)?;
        let payload = &bytes[1..];
        match bytes[0] {
            PERMIT_PUBLIC if payload.iter().all(|b| *b == 0) => Some(Permit::Public),
            PERMIT_BOUND if payload.iter().all(|b| *b == 0) => Some(Permit::Bound),
            PERMIT_IDENTITY => Some(Permit::Identity(Selector::fetch(payload)?)),
            PERMIT_OPENER if payload[8..].iter().all(|b| *b == 0) => {
                Some(Permit::Opener(EntryId::fetch(&payload[..8])?))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::identity::{CoalitionId, PrincipalId};
    use env::{TaskId, wire::Field};

    #[test]
    fn permits_preserve_authority_and_slot() {
        let authority = TaskId::new(23);
        let principal = PrincipalId::new(authority, u64::MAX);
        let coalition = CoalitionId::new(authority, 0);
        for permit in [
            Permit::Public,
            Permit::Bound,
            Permit::Identity(Selector::Exact(principal)),
            Permit::Identity(Selector::DescendantOf(principal)),
            Permit::Identity(Selector::MemberOf(coalition)),
            Permit::Opener(EntryId::new(0)),
        ] {
            let mut bytes = [0u8; Permit::WIDTH];
            permit.store(&mut bytes);
            assert_eq!(Permit::fetch(&bytes), Some(permit));
            assert_eq!(Permit::fetch(&bytes[..bytes.len() - 1]), None);
        }
    }

    #[test]
    fn permit_rejects_unknown_tag_and_unused_payload() {
        let mut bytes = [0u8; Permit::WIDTH];
        bytes[0] = 255;
        assert_eq!(Permit::fetch(&bytes), None);
        for permit in [
            Permit::Public,
            Permit::Bound,
            Permit::Opener(EntryId::new(2)),
        ] {
            permit.store(&mut bytes);
            let last = bytes.len() - 1;
            bytes[last] = 1;
            assert_eq!(Permit::fetch(&bytes), None);
        }
    }
}
