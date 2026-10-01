//! coalition 的**帧那一半** —— 帧与码（内核那一只手的别名在 `protocol` 那一侧的 `mod.rs`）。
//! 本文件**不做裁决**：盟册的规矩全在实现侧那一本账里（`programs/src/service/coalition/core.rs`）。这里只有三件事——
//! 把失败域翻成答话码、把答案编进答话那一格、以及**本族**那几格码 / 记号 / **窗**那一档。

use crate::wire::id::Id;
use crate::wire::message::Message;
use crate::service::principal::PrincipalId;
use env::{Mark, PieToken, TaskId};

pub mod vocab;

pub use self::vocab::*;
impl Id for CoalitionId {
    fn new(raw: usize) -> CoalitionId {
        CoalitionId::new(raw)
    }

    fn get(self) -> usize {
        CoalitionId::get(self)
    }
}

/// 这一枚号在线上是 **8 字节小端**——口径与 `operator::EntryId` 那一处相同（**impl 跟着类型走**，
/// `env` 不认识 [`CoalitionId`]）。读的那一侧**不校验"铸过没有"**：解出来的号在不在盟册里由
/// 核心答（`Fail::Unknown`）。
impl env::wire::Field for CoalitionId {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req {
    /// `FOUND`：铸一枚新盟——**两格都空**。
    Found,
    /// `ENTER`：进 `a` 那一枚盟。
    Enter(CoalitionId),
    /// `LEAVE`：离 `a` 那一枚盟。
    Leave(CoalitionId),
    /// `AMID`：`a` 此刻在 `b` 那一枚盟里吗（**两格都用**）。
    Amid(PrincipalId, CoalitionId),
    /// `BAND`：读 `a` 那一枚盟的盟籍；`b` = **游标**（从哪一枚之后接着读）。
    Band(CoalitionId, Option<PrincipalId>),
    /// `BLOC`：读 `a` 那一位在哪些盟里；`b` = **游标**。
    Bloc(PrincipalId, Option<CoalitionId>),
    /// `ADMIT`：把 `b` 那位放进 `a` 那一枚盟（**只有盟主叫得动**，见 [`Fail::NotChief`]）。
    Admit(CoalitionId, TaskId),
}

impl Req {
    /// 编成线上那一形；`back` = **这一趟的回信孔在对端表里的号**（运输那一格，不是荷载）。
    pub fn query(self, back: PieToken) -> Query {
        let (op, a, b) = match self {
            Req::Found => (FOUND, 0, 0),
            Req::Enter(c) => (ENTER, c.get() as u64, 0),
            Req::Leave(c) => (LEAVE, c.get() as u64, 0),
            Req::Amid(p, c) => (AMID, p.get() as u64, c.get() as u64),
            Req::Band(c, after) => (BAND, c.get() as u64, cursor_of(after)),
            Req::Bloc(p, after) => (BLOC, p.get() as u64, cursor_of(after)),
            Req::Admit(c, target) => (ADMIT, c.get() as u64, target.get() as u64),
        };
        Query { op, a, b, back }
    }
}

/// **收进来的一问**（那两格号已经解成模型类型 / 游标）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Found,
    Enter(CoalitionId),
    Leave(CoalitionId),
    Amid(PrincipalId, CoalitionId),
    Band(CoalitionId, Option<PrincipalId>),
    Bloc(PrincipalId, Option<CoalitionId>),
    /// 代报名：盟号 ＋ **目标那一枚 TID**（解名在服务端）。
    Admit(CoalitionId, TaskId),
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
            FOUND => Some(Wire::Found),
            ENTER => Some(Wire::Enter(CoalitionId::new(q.a as usize))),
            LEAVE => Some(Wire::Leave(CoalitionId::new(q.a as usize))),
            AMID => Some(Wire::Amid(
                PrincipalId::new(q.a as usize),
                CoalitionId::new(q.b as usize),
            )),
            BAND => Some(Wire::Band(
                CoalitionId::new(q.a as usize),
                cursor_in(q.b).map(|raw| PrincipalId::new(raw)),
            )),
            BLOC => Some(Wire::Bloc(
                PrincipalId::new(q.a as usize),
                cursor_in(q.b).map(|raw| CoalitionId::new(raw)),
            )),
            ADMIT => Some(Wire::Admit(
                CoalitionId::new(q.a as usize),
                TaskId::new(q.b as usize),
            )),
            // 表外的动作码：这一码不是我的（但"往哪回"读得出来）。
            _ => None,
        };
        Some((ask, q.back))
    }
}

/// 游标那一格：**`b` = 游标 + 1**，`0` = 没有游标（从头取）。
/// 加一是那个双射：零号是真格子（`PrincipalId::ROOT` 是 0），拿 0 当"没有"会把它漏掉。
pub fn cursor_of<T: Id>(after: Option<T>) -> u64 {
    match after {
        Some(at) => at.get() as u64 + 1,
        None => 0,
    }
}

/// 游标那一格解回来（`0` ⇒ `None`；其余 ⇒ 裸号）。
pub fn cursor_in(b: u64) -> Option<usize> {
    if b == 0 { None } else { Some(b as usize - 1) }
}

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「窗」那一形的**头三格**：状态 ＋ 未完 ＋ 条数（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
/// **"未完"那一格为什么只此一族有**：盟籍没有上限（一格盟可以有很多人）⇒ 窗装不下是常态；
/// 对照 operator 那一侧：一条 pane 本来就不超过 `PANE_CAP`，故那边不用带。
/// **`more` 那一格是真 `bool`**：只许 0 / 1 这条判据收在 [`env::wire::Field`] 一处
/// （`bool` 那一格），本族不再手写一遍、也没有"畸形的 2"这一形可读。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeqHead {
    pub status: u8,
    pub more: bool,
    pub count: u8,
}

/// 一答的上界：**最大那一形**（窗：头三格 ＋ [`WINDOW_CAP`] 枚号）。
pub const UNION_LEN: usize = SeqHead::LEN + WINDOW_CAP * 8;

/// 一窗号的**荷载**（编的那一侧用）：未完那一格 ＋ 一串**裸 8 字节号**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seq {
    more: bool,
    len: usize,
    ids: [u64; WINDOW_CAP],
}

impl Seq {
    fn of<T: Id>(window: &Window<T>) -> Seq {
        let mut ids = [0u64; WINDOW_CAP];
        for (slot, id) in ids.iter_mut().zip(window.iter()) {
            *slot = id.get() as u64;
        }
        Seq {
            more: window.more(),
            len: window.len(),
            ids,
        }
    }

    fn ids(&self) -> &[u64] {
        self.ids.get(..self.len).unwrap_or(&[])
    }

    /// 按**问的那一族**把裸号造回来（读那一侧；编那一侧是 `of`）。
    pub fn window<T: Id>(&self) -> Window<T> {
        Window::gather(
            self.more,
            self.ids().iter().map(|raw| T::new(*raw as usize)),
        )
    }
}

/// **一答的形状**——三形：格状态（1）／一格答（10）／一窗号（`3 + 8n`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Union {
    /// 失败那几格（[`UNKNOWN`] / [`FULL`] / [`BAD`]）。
    Status(u8),
    /// 一格答：一枚号 / 是或非。
    One(Reply),
    /// 一窗号。
    Seq(Seq),
}

impl Union {
    pub fn seq<T: Id>(window: &Window<T>) -> Union {
        Union::Seq(Seq::of(window))
    }
}

impl Message for Union {
    type In = Union;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match *self {
            Union::Status(code) => Status { status: code }.store_at(out, 0),
            Union::One(reply) => reply.store_at(out, 0),
            Union::Seq(seq) => {
                let head = SeqHead {
                    status: OK,
                    more: seq.more,
                    count: seq.len as u8,
                };
                let at = head.store_at(out, 0)?;
                env::wire::store_tail(out, at, seq.ids())
            }
        }
    }

    /// 解一答：**先按长度分那一形**，再在该形自己的判据里解——不自洽 ⇒ `None`（不猜、不崩）。
    fn fetch(bytes: &[u8]) -> Option<Union> {
        match bytes.len() {
            Status::LEN => Some(Union::Status(Status::fetch(bytes)?.status)),
            Reply::LEN => Some(Union::One(Reply::fetch(bytes)?)),
            len if (SeqHead::LEN..=UNION_LEN).contains(&len) => {
                let head = SeqHead::fetch(bytes)?;
                let more = head.more;
                let count = head.count as usize;
                if count > WINDOW_CAP {
                    return None;
                }
                let body = bytes.get(SeqHead::LEN..)?;
                let mut ids = [0u64; WINDOW_CAP];
                let end = env::wire::fetch_tail(body, 0, &mut ids[..count])?;
                // **帧长即条数**：对不上就是不认（短一字节、条数说谎都落在这一句上）。
                if end != body.len() {
                    return None;
                }
                Some(Union::Seq(Seq {
                    more,
                    len: count,
                    ids,
                }))
            }
            _ => None,
        }
    }
}

const _: () = assert!(BACK.get() != Mark::NONE.get());

const _: () = assert!(BACK.get() != Mark::of(NAME).get());
