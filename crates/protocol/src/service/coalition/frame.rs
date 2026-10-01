//! coalition 的**帧那一半** —— 帧与码（内核那一只手的别名在 `protocol` 那一侧的 `mod.rs`）。
//! 本文件**不做裁决**：盟册的规矩全在实现侧那一本账里（`programs/src/system/coalition/core.rs`）。这里只有三件事——
//! 把失败域翻成答话码、把答案编进答话那一格、以及**本族**那几格码 / 记号 / **窗**那一档。

use crate::wire::id::Id;
use crate::wire::message::Message;
use crate::service::principal::PrincipalId;
use env::{Mark, PieToken, TaskId};

use crate::common::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CoalitionId(usize);

impl CoalitionId {
    /// 由裸号造一个（线上解码面；没铸过的号从这里进来）。
    pub const fn new(raw: usize) -> CoalitionId {
        CoalitionId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

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

/// 失败域：**三格**，每格一个**不同的下一步**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 这枚盟不存在（从来没铸过），或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**
    /// 或**我还没有身份**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    Full,
    /// 你手里那一枚门牌给不了这一条：**换一枚**（或换一位客人），别重试。
    Denied,
    /// **你不是这一枚盟的盟主** ⇒ 别拿它来代报名（要改的是"换一条路"，不是"再试一次"）。
    NotChief,
}

/// 一窗最多几枚号。条数是策略、容器要有界 ⇒ 窗口有顶，**"还有没有"由 `more` 说**。
pub const WINDOW_CAP: usize = 16;

/// 一窗号：**一趟读的读数**（最多 [`WINDOW_CAP`] 枚，**号序升序**）。
/// 空位是 `None` 而不是 `T::new(0)`：**零号是真格子**（`PrincipalId::ROOT` 就是 0），
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window<T: Id> {
    items: [Option<T>; WINDOW_CAP],
    n: usize,
    more: bool,
}

impl<T: Id> Window<T> {
    /// 空的那一串（`more = false`）。
    pub const fn new() -> Window<T> {
        Window {
            items: [None; WINDOW_CAP],
            n: 0,
            more: false,
        }
    }

    /// 由一串号凑一窗（`more` = 窗外还有）——**解码面**：线上收来的那一窗由这里成形。
    /// 收够 [`WINDOW_CAP`] 枚就停：帧长了是帧的毛病，读的人只认窗前这些（帧长与条数对不对
    /// 由 `protocol` 那一侧的 `frame` 那一层先挡掉）。
    pub fn gather(more: bool, ids: impl Iterator<Item = T>) -> Window<T> {
        let mut out = Window::new();
        for id in ids.take(WINDOW_CAP) {
            out.push(id);
        }
        out.more = more;
        out
    }

    /// 几枚。
    pub fn len(&self) -> usize {
        self.n
    }

    /// 窗外还有没有（这一趟没答完的那些）。
    pub fn more(&self) -> bool {
        self.more
    }

    /// 第 `at` 枚（号序；越界 ⇒ `None`）。
    pub fn get(&self, at: usize) -> Option<T> {
        if at < self.n {
            self.items.get(at).copied().flatten()
        } else {
            None
        }
    }

    /// 号序走一遍。
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.items[..self.n].iter().filter_map(|slot| *slot)
    }

    /// 末一枚——**它就是下一页的游标**（空窗 ⇒ `None`）。
    pub fn last(&self) -> Option<T> {
        self.n.checked_sub(1).and_then(|at| self.get(at))
    }

    /// **取窗那一侧用**：收一枚。收下了 ⇒ `true`；**已经满了** ⇒ `false` 并点亮
    /// [`Window::more`]（"这一趟没答完"）。
    pub fn put(&mut self, id: T) -> bool {
        if self.full() {
            self.more = true;
            return false;
        }
        self.push(id);
        true
    }

    /// 收一枚（再满就丢：取窗那边收了 [`WINDOW_CAP`] 枚就停）。
    fn push(&mut self, id: T) {
        if let Some(slot) = self.items.get_mut(self.n) {
            *slot = Some(id);
            self.n += 1;
        }
    }

    /// 装满了。
    fn full(&self) -> bool {
        self.n == WINDOW_CAP
    }
}

/// 七条线上动作——**与核心那七条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
pub const FOUND: u8 = 1;
pub const ENTER: u8 = 2;
pub const LEAVE: u8 = 3;
pub const AMID: u8 = 4;
pub const BAND: u8 = 5;
pub const BLOC: u8 = 6;
/// **代报名**：把**另一位**放进盟主自己立的那一枚盟。
pub const ADMIT: u8 = 7;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::wire::fail_codes::OK;

/// 答话那一格：失败域那三格 + "读不懂"。
/// [`BAD`] 在失败表外（同板 / 树 / 身份服务那三家的先例）：它不是"哪个协议说的事"，
/// 是**这一问读不懂**。
pub const UNKNOWN: u8 = 1;
pub const FULL: u8 = 2;
pub const BAD: u8 = 3;
pub const DENIED: u8 = 4;
/// 你不是这一枚盟的盟主：代报名只有**立它那位**做得成。
pub const NOT_CHIEF: u8 = 5;

// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——coalition 与 principal 同形（这一族
// 的帧就是照它立的），故只有一份；这里只按本族的名字转出来（`mod.rs` 那一句
// 点名转出照旧，调用点一处都不用改）。

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

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    /// 四格：每格语义见各自的注。
    /// 数字按本族失败域的顺序排（`BAD` 收尾且在表外）——别家同一个概念排的是别的号，那不是约定。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Full => FULL,
    Fail::Denied => DENIED,
    Fail::NotChief => NOT_CHIEF,
}

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
/// 与另几面的 `*-back` 同一个形状、不同的记号：同一张表里两面的回信孔若刻同一个记号，
/// 就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("coalition-back");

/// **本族那块窗格在树上的路**：`/svc/sys/coalition`（头两段是四族共用的
/// [`crate::common::svc::DIR`]，末段是本族自己的名字 [`NAME`]）——**一处说全**（同 principal）。
pub const DIR: &Path = Path::new("svc/sys/coalition");

/// 本服务在树上的那一段名字：`/svc/sys/coalition`——**它不是一格**（
/// 两枚门牌是它底下那两格 `/svc/sys/coalition/{ask,set}`，末段名由
/// [`Grant::name`](super::grant::Grant::name) 给）。
pub const NAME: &str = "coalition";

const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
