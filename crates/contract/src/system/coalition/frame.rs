//! coalition 的**帧那一半** —— 帧与码（内核那一只手的别名在 `protocol` 那一侧的 `mod.rs`）。
//!
//! 本文件**不做裁决**：盟册的规矩全在 [`core`](super::core)。这里只有三件事——
//! 把失败域翻成答话码、把答案编进答话那一格、以及**本族**那几格码 / 记号 / **窗**那一档。
//!
//! **照实记（这一份为什么拆出来）**：见 `principal/frame.rs` 的同一条——帧形的边角机器走不到，
//! 拆开是为了让它们在**宿主靶**上编得动；**那台靶已删**（用户裁定"protocol-case 没必要"）⇒
//! 这一份照旧只认 `env` 与同层 `core`，但那些边角今天**没有判据**。
//!
//! # 帧（与 `system::principal::frame` 同一形状；窗那一档多一种答形）
//!
//! ```text
//!   Query  [0] op   [1..9] a   [9..17] b   [17..25] back     25
//!   Reply  [0] status  [1] flag  [2..10] a                   10（`crate::frame` 那一份）
//!          [0] status                                        1（失败那几格）
//!          [0] status  [1] 未完  [2] 条数  [3..] 号           3 + 8n，上界 [`UNION_LEN`] = 131
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`FOUND` 两格都空，`ENTER` / `LEAVE` 只用 `a`，
//! `AMID` 两格都用，`BAND` / `BLOC` 的 `b` 是**游标**）——而"这一条有几格"由下面的
//! [`Req`] / [`Wire`] 按类型说。答话的**一格答**那一形本体在 [`crate::frame`]（两族同形，故
//! 只有一份）；**格状态 ＋ 窗**那两档留在这里（只此一族），三形合成 [`Union`]——**形状由长度分**，
//! 故写法与读法是同一个。
//!
//! **照实记（这一行原写"一问 17 字节"）**：那是 `back` 那一格落地之前抄的，此后一问一直是
//! `1 + 8 + 8 + 8 = 25`（同 `principal/frame.rs` 那条，详见 [`crate::frame`]）。
//!
//! **游标是阈值，说在 `b` 那一格**：`b = 游标 + 1`，`0` = 没有游标（从头取）。加一是有理的
//! ——**零号是真格子**（`PrincipalId::ROOT` 是 0、`CoalitionId(0)` 是一枚普通的盟），
//! 拿 0 当"没有"会把那一位漏掉。取的是**号 > 阈值**的那些，故没有"过期游标"这回事。
//!
//! **报文里没有"我是谁"这一格**：发送者由内核在 `Push` 那一刻盖章，Server 拿去名册问。
//!
//! # 编答的助手**少一个**
//!
//! `system::principal::frame` 有 `reply_present`（"有没有一条号"），这里不需要——
//! 本族没有"可能没有的一条号"那种答案（`found` 必有号，`amid` 是是非）。**帮手少一个，
//! 是原语少一条的余数。**
//!
//! # 码的数字**不照抄 principal**
//!
//! 同一个概念 `UNKNOWN`，operator 那一面是 1、principal 那一面是 2——三家各按**自己失败域
//! 的顺序**排、`BAD` 收尾。故本族按自己的两格排（见 `fail_codes!` 那张表）：照抄别家只会
//! 让自己表里空出一个号。

use super::core::{CoalitionId, Fail, WINDOW_CAP, Window};
use crate::id::Id;
use crate::message::Message;
use crate::system::principal::core::PrincipalId;
use env::{Mark, PieToken};

// ── 码 ──────────────────────────────────────────────────────

/// 六条线上动作——**与核心那六条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
pub const FOUND: u8 = 1;
pub const ENTER: u8 = 2;
pub const LEAVE: u8 = 3;
pub const AMID: u8 = 4;
pub const BAND: u8 = 5;
pub const BLOC: u8 = 6;

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格：失败域那两格 + "读不懂"。
///
/// [`BAD`] 在失败表外（同板 / 树 / 身份服务那三家的先例）：它不是"哪个协议说的事"，
/// 是**这一问读不懂**。
pub const UNKNOWN: u8 = 1;
pub const FULL: u8 = 2;
pub const BAD: u8 = 3;

// ── 帧骨架（两族同形的那一份）───────────────────────────────
//
// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——coalition 与 principal 同形（这一族
// 的帧就是照它立的），故只有一份；这里只按本族的名字转出来（`mod.rs` 那一句
// 点名转出照旧，调用点一处都不用改）。

pub use crate::frame::{Query, Reply};

// ── 一问：一条动作一格 ──────────────────────────────────────

/// **一问的形状**——一条动作一格（同 `principal/frame.rs` 那条照实记：它替掉了
/// `pack_ask(op, a, b, back)` 那种"任何一枚码配上任何两格数"）。
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
            // 表外的动作码：这一码不是我的（但"往哪回"读得出来）。
            _ => None,
        };
        Some((ask, q.back))
    }
}

// ── 窗：游标与一窗号 ────────────────────────────────────────

/// 游标那一格：**`b` = 游标 + 1**，`0` = 没有游标（从头取）。
///
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

// ── 一答：三种形状（格状态 ＋ 一格答 ＋ 一窗号）──────────────

/// 「格状态」那一形：失败那几格（[`UNKNOWN`] / [`FULL`] / [`BAD`]）只有这一格。
///
/// **照实记（为什么这一族多出这一形）**：成功那两形都带回荷载，失败没有——故线上有三种长度
/// （1 / 10 / `3 + 8n`），客侧按"我问的是哪一条"认。principal 那一面没有这一形：它的失败也占满
/// 10 字节（`Reply` 那一形）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「窗」那一形的**头三格**：状态 ＋ 未完 ＋ 条数（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
///
/// **"未完"那一格为什么只此一族有**：盟籍没有上限（一格盟可以有很多人）⇒ 窗装不下是常态；
/// 对照 operator 那一侧：一条 pane 本来就不超过 `PANE_CAP`，故那边不用带。
///
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
///
/// **照实记（为什么存裸号、不存 `PrincipalId` / `CoalitionId`）**：两个号空间在这一格上
/// **分不开**（`band` 取的是身份号、`bloc` 取的是盟号，线上逐字同形），而 [`Union`] 得是**一枚
/// 具体类型**（服务端一处收尾：装一条、发一条）⇒ 不能按号泛型。与 operator 那格 `Word` 同一条
/// 口径：字段只管这一格多宽、怎么落字节，含义归问的人认。
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
///
/// **照实记（名字）**：用户裁定这一族与树那一族同名同位——答的**形状**那一面叫 `Union`（同
/// `board::Req` 那句"一问的形状"，写成 `enum` 就是"几样里的一件"）。一格答那一形 [`Reply`]
/// 的本体在 [`crate::frame`]（两族同形）。
///
/// **照实记（这一族没有一个"原样的字节"读面——树那一族有）**：树那一族的四形**在线上分不开**
/// （"名"那一条长度即名长、另几形都以状态起头），故它把字节原样收下、由问的人认。这一族**不用
/// 那一手**：三形的长度互不相撞（`1` / `10` / `3 + 8n`，第三族全是 ≡ 3 mod 8，`n = 0..16`）⇒
/// **长度一说，形状就定了**。故 `In = Union`——与板那一族、与 [`Reply`] 同一条"写法与读法是
/// 同一个"。
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
    /// 编一答：一窗号（**裸号进窗**——见 [`Seq`] 那条照实记）。
    pub fn seq<T: Id>(window: &Window<T>) -> Union {
        Union::Seq(Seq::of(window))
    }
}

impl Message for Union {
    /// **写法与读法是同一个**：形状由长度分得开（见 [`Union`] 那条照实记）。
    type In = Union;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match *self {
            Union::Status(code) => Status { status: code }.store_in(out),
            Union::One(reply) => reply.store_in(out),
            Union::Seq(seq) => {
                let head = SeqHead {
                    status: OK,
                    more: seq.more,
                    count: seq.len as u8,
                };
                let at = head.store_in(out)?;
                env::wire::store_tail(out, at, seq.ids())
            }
        }
    }

    /// 解一答：**先按长度分那一形**，再在该形自己的判据里解——不自洽 ⇒ `None`（不猜、不崩）。
    ///
    /// **照实记（从前那两条读法的次序搬进了这里，结果逐条相同）**：客侧那两条路各有一套次序
    /// ——`raw` 那条**先判长度**（恰好 [`Reply::LEN`] 才往下走，故一格状态那一形在那条路上读不出
    /// `FULL`），`read_seq` 那条**先看码**（非 `OK` ⇒ 那一格码，`FULL` 一路走到 `Fail::Full`）。
    /// 长度一分，两条次序都落在下面：**一格答那一形只认恰好 10**（`Status(FULL)` 落不进它），而
    /// **窗那一形先看码**（`Status(FULL)` 由它读成那一格码）。客侧那两条路各自认自己那一形
    /// （见 `coalition/client.rs`），判据一字未改。
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

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// 两格，**没有 `Denied`**：盟无主，没有一处"你得请谁来做"的判断。数字按本族失败域的
    /// 顺序排（`BAD` 收尾且在表外）——别家同一个概念排的是别的号，那不是约定。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Full => FULL,
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
///
/// 与另几面的 `*-back` 同一个形状、不同的记号：同一张表里两面的回信孔若刻同一个记号，
/// 就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("coalition-back");

/// 树上那块窗格的名字（门牌的第一段）：`/sys`。
pub const DIR: &str = "sys";

/// 本服务在树上的名字（门牌的第二段）：`/sys/coalition`。
pub const NAME: &str = "coalition";

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台那条 `the_three_back_marks_of_the_three_doors_do_not_collide`（那条判据随宿主靶
// 一并删了，用户裁定"protocol-case 没必要"）；**与名册那一对**钉在
// `crate::system::principal::frame`，**与线那一对**钉在 `lib.rs`——线那一枚住在
// `driver::line::frame`，而这一份**只认得 `env` 与同层 `core`**，看不见 `driver`。
const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
