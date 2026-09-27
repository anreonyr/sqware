//! control 的**帧那一半** —— 帧、码、记号、状态。
//!
//! ```text
//!   Ask    [0] op  [1..33] name  [33..41] back      41（表求和：`Ask::LEN`）
//!   Said   [0] status  [1] state                     2（表求和：`Said::LEN`）
//! ```
//!
//! **帧里没有镜像**（见 [`super`] 的"`build` 不拷字节"那一节）：`mint` 那一问只带名字，
//! 持表那一侧自己去清单里取那段 `&[u8]`。**帧里也没有"几格通道"/"要什么资源"**：那是
//! `start` / `wire` 的装配细节，归实现侧按那一台的 `setup` 推。
//!
//! 本文件**不做裁决**：生命周期的规矩在那张表里（`programs/src/system/desk.rs` 与
//! `programs/src/system/control/`）。这里只有：失败域 ↔ 答话码、状态 ↔ 那一格数、
//! 一问一答两张表、以及本族那几格记号。
//!
//! # 四个动作在报文里的码
//!
//! `MINT` / `START` / `STOP` / `STATE`——**与四手同名**：线上与模型是同一件事的两层，
//! 不该各起一套词（同 board / operator 那两族的纪律）。

use env::{Mark, Name, PieToken};

use crate::message::Message;

// ── 状态 ────────────────────────────────────────────────────

/// Service 的生命阶段。**失败不在这里**——失败由 [`Fail`] 承载（两者是两件事）。
///
/// 五格与 `programs/src/system/desk.rs` 的 `State` 逐格对应，且**只描述实例的生命阶段**：
/// "有界预算试几次"、"放弃之后算什么"都是 Server 的策略，不在这里另立一格（那一笔账见
/// `crates/protocol/src/system/mod.rs` 的"预算与放弃"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// 表里有这一行，但还没起过。
    NeverStarted,
    /// 起了，正在等它宣布就绪。
    Starting,
    /// 已就绪。
    Ready,
    /// 已下令收，还没确认收干净。
    Stopping,
    /// 起过、现在是死的（被收掉或自己走的）。
    Dead,
}

impl State {
    /// 线上那一格数（**判别值**：`State` 的码与内核无关，是本协议自己的）。
    pub const fn code(self) -> u8 {
        match self {
            State::NeverStarted => 0,
            State::Starting => 1,
            State::Ready => 2,
            State::Stopping => 3,
            State::Dead => 4,
        }
    }

    /// 那一格数 → 状态。表外的数 ⇒ `None`（**不猜**）。
    pub const fn of_code(code: u8) -> Option<State> {
        match code {
            0 => Some(State::NeverStarted),
            1 => Some(State::Starting),
            2 => Some(State::Ready),
            3 => Some(State::Stopping),
            4 => Some(State::Dead),
            _ => None,
        }
    }
}

// ── 失败域 ──────────────────────────────────────────────────

/// 失败域：五格，**前四格各对应一个不同的下一步**（照实抄 `programs/src/system/core.rs` 那四格）。
///
/// 它是**协议这一侧**的名字：调度侧那四格是 `Unknown` / `BadImage` / `Full` / `NotReady`，
/// 与这里逐格同形——两份不是"抄一遍"，是同一件事的两层（模型那一份不碰 `runtime`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 表里没这个名字，或它已经登记过。
    Unknown,
    /// 镜像装不上（内核 `UnitFail::BadImage`）。
    BadImage,
    /// 表满，或线程 / 帧产不出来（内核 `UnitFail::OoM`）。
    Full,
    /// 没就绪：等到期还没起来、半路死了、或此刻不该起（已在跑）。
    NotReady,
    /// **本端读不懂那一句**（帧坏了 / 答话那一格解不动 / 期限到了还没答）。
    ///
    /// 它在**失败表外**（同板、树那两族的先例）：它不是"持表那一侧说的事"，是**这一问没走到**。
    /// 对本端而言与"这条路别指望了"同一个下一步，故不往 [`Fail`] 的语义格里塞。
    Bad,
}

// ── 帧 ──────────────────────────────────────────────────────

/// 一问：动作码 ＋ 名字 ＋ **回信孔那一格**。
///
/// `back` 是**运输**那一格（往哪回），不是动作的荷载——它排最后，谁都不许把它当第二个名字使。
/// 这一格是"我给你的那一枚在你表里是几号"：对端据此一次 `Reserve` 就认出回信的路，不必扫表
/// （理由与实测见 [`crate::frame::Query`] 那条照实记）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ask {
    pub op: u8,
    /// 这一条服务的名字（清单名，≤ 31 字节）。
    pub name: Name,
    pub back: PieToken,
}

/// 答话那一格：状态 ＋ 答案那一格（`state` 那一问用它装 [`State`] 的判别值）。
///
/// 定长 2 字节：**每一问都只需要这两格**——`mint` / `start` / `stop` 只看状态，`state` 再看
/// 第二格。故不需要一变多形（operator 那一族才有"一答多形"的麻烦）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    pub status: u8,
    pub a: u8,
}

impl Message for Said {
    /// **写法与读法是同一个**：这一形两格俱全，读的人不必再问"我问的是哪一条"。
    type In = Said;
    /// 定长一答（[`Said::LEN`]）。
    type Buf = [u8; Said::LEN];
    const EMPTY: Self::Buf = [0u8; Said::LEN];

    /// 表那一手 `store_in`（写更大的缓冲、返长度）——正是这一手要的；表上那枚**同名**的 `store`
    /// 要的是定长数组、返 `()`，两回事（同 `crate::frame::Reply` 那一格）。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Said::store_in(self, out)
    }

    /// **恰好 2 字节**：长一字节、短一字节都是读不懂（同板、树那两族那条照实记）。
    fn fetch(bytes: &[u8]) -> Option<Said> {
        if bytes.len() != Said::LEN {
            return None;
        }
        Said::fetch(bytes)
    }
}

// ── 四个动作的码 ────────────────────────────────────────────

const MINT: u8 = 1;
const START: u8 = 2;
const STOP: u8 = 3;
const STATE: u8 = 4;

/// 成功那一格：**全协议同一个号**——定义在 `crate::fail_codes`，本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格。**前四格与 [`Fail`] 的语义格一一对应**，第五格不是失败域的：这一问读不懂。
///
/// 数字是**线上的**；[`Fail`] 是模型那一侧的名字，两者的对照表只此一份（持表那一侧编、
/// 客人那一侧读）。
pub const UNKNOWN: u8 = 1;
pub const BADIMAGE: u8 = 2;
pub const FULL: u8 = 3;
pub const NOTREADY: u8 = 4;
pub const BAD: u8 = 5;

/// 失败域 → 答话那一格。`None`（没失败）⇒ [`OK`]。
///
/// **它不套 `fail_codes!` 宏**（照实记）：本族是五格，而 [`Fail::Bad`] 由**本端**产生
/// （推不动 / 读不到 / 解不动）——对端从来说不出这句话。若把它也列进宏那张双射表，宏会为
/// `BAD` 生成一个反向支，而**反向读到的任何表外码本来就要落回 [`Fail::Bad`]** ⇒ 那一支要么
/// 永远不可达、要么把"读不懂"说成"对端答了 Bad"。两句话都假，故这一对由人写全：
/// 正向五格齐全（模型 → 线上只有一处映射），反向把表外一律折成 [`Fail::Bad`]。
pub const fn fail_to_code(fail: Option<Fail>) -> u8 {
    match fail {
        None => OK,
        Some(Fail::Unknown) => UNKNOWN,
        Some(Fail::BadImage) => BADIMAGE,
        Some(Fail::Full) => FULL,
        Some(Fail::NotReady) => NOTREADY,
        Some(Fail::Bad) => BAD,
    }
}

/// 线上答话那一格 → 失败域。`OK` ⇒ `None`；**表外与 [`BAD`] 都折成 [`Fail::Bad`]**。
///
/// 那两格不是同一件事（"读不懂这一句"与"对端说它读不懂"），但对**本端**是同一个下一步：
/// 这一趟别指望了。分得开它们的那一格在对端（同 principal / coalition 那两族那条照实记）。
pub const fn code_to_fail(code: u8) -> Option<Fail> {
    match code {
        OK => None,
        UNKNOWN => Some(Fail::Unknown),
        BADIMAGE => Some(Fail::BadImage),
        FULL => Some(Fail::Full),
        NOTREADY => Some(Fail::NotReady),
        _ => Some(Fail::Bad),
    }
}

/// **一问的形状**——一条动作一格：荷载只有名字，"回信往哪"由 [`Ask::back`] 带。
/// 照实记（它替掉了什么）：从前那种 `pack_ask(op, name, …)` 里**任何一枚码都能配上任何一份
/// 荷载**，错配编得过；今天形状由类型说，编解码由表生成（同 board / principal 那两族的刀）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req {
    /// `MINT`：按名字起一条（建域 + 产代表线程，**恒产未放行**）。
    Mint(Name),
    /// `START`：放行 ＋ 等就绪。
    Start(Name),
    /// `STOP`：下令收掉（下令即回）。
    Stop(Name),
    /// `STATE`：这一条此刻处于哪个阶段。
    State(Name),
}

impl Req {
    /// 编成线上那一形；`back` = **这一趟的回信孔在对端表里的号**（运输那一格，不是荷载）。
    pub fn ask(self, back: PieToken) -> Ask {
        let (op, name) = match self {
            Req::Mint(name) => (MINT, name),
            Req::Start(name) => (START, name),
            Req::Stop(name) => (STOP, name),
            Req::State(name) => (STATE, name),
        };
        Ask { op, name, back }
    }
}

/// **收进来的一问**。
///
/// 两格失败分得开（同 [`crate::frame::Query`] 那条）：**长度不对** ⇒ 外层 `None`（连"往哪回"
/// 都没有 ⇒ 不动表、也不回话）；**动作码不认得** ⇒ 内层 `None`（这一问有回信的路，只是这一码
/// 我不认 ⇒ 回一句 [`BAD`]）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Mint(Name),
    Start(Name),
    Stop(Name),
    State(Name),
}

impl Wire {
    /// 解一问：`(读出来的动作, 回信孔那一格)`。
    pub fn take(bytes: &[u8]) -> Option<(Option<Wire>, PieToken)> {
        if bytes.len() != Ask::LEN {
            return None;
        }
        let q = Ask::fetch(bytes)?;
        let ask = match q.op {
            MINT => Some(Wire::Mint(q.name)),
            START => Some(Wire::Start(q.name)),
            STOP => Some(Wire::Stop(q.name)),
            STATE => Some(Wire::State(q.name)),
            _ => None,
        };
        Some((ask, q.back))
    }
}

/// 编一答：只有状态那一格（失败，或读不懂）。
pub const fn said_status(status: u8) -> Said {
    Said { status, a: 0 }
}

/// 编一答：`OK` ＋ 一个 [`State`]（只有 `state` 那一问用）。
pub const fn said_state(state: State) -> Said {
    Said {
        status: OK,
        a: state.code(),
    }
}

/// 解一答：`Ok((状态, 答案那一格))`；**长度不对 ⇒ `None`**（读不动的答话）。
pub fn said_read(bytes: &[u8]) -> Option<(u8, u8)> {
    let said = Said::fetch(bytes)?;
    Some((said.status, said.a))
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────

/// 这条路叫什么（泊位那一格）：**两侧同一个**。
pub const LINK: &str = "control";

/// 这一面在树上的名字（挂到 `/sys/control`）：**与 [`LINK`] 同一个串**——"泊位叫 `control`"
/// 与"它挂在哪一格"是同一件事的两层，重名不是重名。
pub const NAME: &str = "control";

/// 问话孔那一枚上的记号。**带面名**（`control-ask`）：认领键是"谁开的 + 记号"，而同一枚任务
/// 可能同时是两面的客人——两枚孔都铸在它自己那张表里，记号再一样就分不开（理由与实测见
/// `system::operator::frame::ASK_MARK`）。
pub const ASK_MARK: Mark = Mark::of("control-ask");

/// 回信孔的记号：客人**每趟**铸一枚、借给对端（这一趟的答话从它回来）。
pub const BACK: Mark = Mark::of("control-back");

/// 树上的目录名（门牌的第一段）：`/sys`——与 principal / coalition 共用同一块窗格。
pub const DIR: &str = "sys";

// ── 面不相撞（**编译期**钉住——照 principal / coalition 那两族的先例）─────────────

const _: () = assert!(ASK_MARK.get() != Mark::NONE.get());
const _: () = assert!(ASK_MARK.get() != BACK.get());
const _: () = assert!(ASK_MARK.get() != Mark::of(LINK).get());
const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(LINK).get());
