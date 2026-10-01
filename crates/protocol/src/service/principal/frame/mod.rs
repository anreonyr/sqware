//! principal 的**帧那一半** —— 帧与码（内核那两只手的别名在 `protocol` 那一侧的 `mod.rs`）。

use crate::wire::id::Id;
use env::{Mark, PieToken, TaskId};

pub mod vocab;

pub use self::vocab::*;
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

/// 成功那一格：**全协议同一个号**——定义在 [`crate::wire::OK`]，本族只把它转出来
/// （[`crate::WireCodes`] 派生的两向读法就是拿它当"没失败"那一格）。
pub use crate::wire::OK;

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

const _: () = assert!(BACK.get() != Mark::NONE.get());

const _: () = assert!(BACK.get() != Mark::of(NAME).get());

const _: () = assert!(BACK.get() != crate::service::coalition::frame::BACK.get());
