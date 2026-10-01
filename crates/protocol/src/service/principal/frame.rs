//! principal 的**帧那一半** —— 帧与码（内核那两只手的别名在 `protocol` 那一侧的 `mod.rs`）。

use crate::wire::id::Id;
use env::{Mark, PieToken, TaskId};

use crate::common::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PrincipalId(usize);

impl PrincipalId {
    /// 根：Server 启动时自带的那一枚，**唯一没有父的节点**。
    pub const ROOT: PrincipalId = PrincipalId(0);

    /// 由裸号造一个（线上解码面；树外的号从这里进来）。
    pub const fn new(raw: usize) -> PrincipalId {
        PrincipalId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Id for PrincipalId {
    fn new(raw: usize) -> PrincipalId {
        PrincipalId::new(raw)
    }

    fn get(self) -> usize {
        PrincipalId::get(self)
    }
}

/// 这一枚号在线上是 **8 字节小端**——口径与 `operator::EntryId` 那一处相同（**impl 跟着类型走**，
/// `env` 不认识 [`PrincipalId`]）。读的那一侧**不校验"还在不在"**：解出来的号在不在谱系里由
/// 核心答。
impl env::wire::Field for PrincipalId {
    const WIDTH: usize = 8;

    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(Self::from_bytes(bytes.get(..8)?.try_into().ok()?))
    }
}

/// 失败域：三格，每格一个**不同的下一步**。
/// **`Resolve` 与三条谱系读没有失败域**——读是公开的（答案不是秘密，Principal 不授予任何
/// 东西）；这里三格只被写的那两条与"查无此节点"用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 你不是那一个：不是写名册的那一枚（`Bind`/`Unbind`）、不是"当前正好代表 `p`"的那一枚
    /// （`derive`）、或目标不在**你自己那一支**里（`adopt`）。调用方要改的是：**该请谁来做**
    /// 或**换一个目标**。
    Denied,
    /// 这条 PrincipalId 不在树里，或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    Full,
}

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
pub const DROP: u8 = 8;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::wire::fail_codes::OK;

/// 答话那一格：失败域那几格 + "读不懂"。
/// [`BAD`] 在失败表外（同板/树的先例）：它不是"哪个协议说的事"，是**这一问读不懂**。
pub const DENIED: u8 = 1;
pub const UNKNOWN: u8 = 2;
pub const FULL: u8 = 3;
pub const BAD: u8 = 4;

// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——principal 与 coalition 同形，故只有
// 一份；这里只按本族的名字转出来（`mod.rs` 那一句点名转出照旧，调用点一处
// 都不用改）。**本族自己的**是下面那些：码、`reply_present`、失败表、记号。

pub use crate::wire::frame::{Query, Reply};

/// **一问的形状**——一条动作一格：`a` / `b` 两格在该动作里有几个就有几个（"只填 a"那几条
/// **再没有第二个号可填**）。
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
    /// `DROP`：转换 · 丢——把当前号置空（也只认"发送者是谁"）。
    Drop,
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
            Req::Drop => (DROP, 0, 0),
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
    Drop,
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
            DROP => Some(Wire::Drop),
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
pub fn reply_present(present: bool, at: PrincipalId) -> Reply {
    Reply {
        flag: present,
        ..Reply::value(at)
    }
}

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    /// **本表只装写的那两条与"查无此节点"**：读的答案（没绑 / 它是根）走 `OK` + `flag`，
    /// 不进这张表（见文件头）。
    bijective Fail; OK;
    Fail::Denied => DENIED,
    Fail::Unknown => UNKNOWN,
    Fail::Full => FULL,
}

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
/// 与 rtc 那一面的 `rtc-back` 同一个形状、不同的记号：两块门牌的回信孔若刻同一个记号，
/// 同一张表里就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("principal-back");

/// **本族那块窗格在树上的路**：`/svc/sys/principal`（头两段是四族共用的
/// [`crate::common::svc::DIR`]，末段是本族自己的名字 [`NAME`]）。
pub const DIR: &Path = Path::new("svc/sys/principal");

/// 本服务在树上的那一段名字：`/svc/sys/principal`——**它不是一格**（
/// 两枚门牌是它底下那两格 `/svc/sys/principal/{ask,set}`，末段名由
/// [`Grant::name`](super::grant::Grant::name) 给）。
pub const NAME: &str = "principal";

const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
const _: () = assert!(BACK.get() != crate::service::coalition::frame::BACK.get());
