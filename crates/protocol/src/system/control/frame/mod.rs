//! 帧、码、记号、状态
//! **帧里没有镜像**（见 super 的"`build` 不拷字节"那一节）：`mint` 那一问只带名字，
//! 持表那一侧自己去清单里取那段 `&[u8]`。**帧里也没有"几格通道"/"要什么资源"**：那是
//! `embark` / `wire` 的装配细节，归实现侧按那一台的 `setup` 推。
//! 一问一答两张表、以及本族那几格记号。
//! # 五个动作在报文里的码
//! `MINT` / `EMBARK` / `DEBARK` / `RUIN` / `STATE`——**与五手同名**：线上与模型是同一件事的两层，
//! 不该各起一套词（同 operator 那一族的纪律）。

use alloc::string::String;
use env::{Mark, PieToken, TaskId};

use crate::wire::message::Message;
use env::wire::Span as _;

pub mod vocab;

pub use self::vocab::*;
/// 一问：动作码 ＋ 名字 ＋ **回信孔那一格**
/// `back` 是**运输**那一格（往哪回），不是动作的荷载——它排最后，谁都不许把它当第二个名字使
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 41)]
pub struct Ask {
    pub op: u8,
    /// 这一条服务的名字（清单名，≤ 31 字节）
    pub name: String,
    pub back: PieToken,
}

/// 答话那一格：状态 ＋ 答案那一格 ＋ **那一条的身子**
/// 定长一形（**不改多变**）：`mint` / `debark` 只看状态，`state` 再看第二格，`embark` 看第三格——
/// 每一问都只需要这三格里属于它的那一格，故不需要 operator 那一族那种"一答多形"
/// # `task` 那一格：身子的 `TaskId` 能跨域，通道副本不能
/// `Embark` 那一答把**子域的 `TaskId`** 交出来——那是"`build` 不拷字节"那条口径的延续
/// 内核按 `TaskId` 认一枚线程，与它在哪个域无关，故这个号**跨域有意义**。而 `Endpoint`
/// 的两枚孔是"持有它的那张表里才念得动"的号（communication
/// 事实 8）⇒ control 铸出来的是**它自己那一侧**的孔，交不到客人手里。故
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    pub status: u8,
    /// State 的判别值（只有 `state` 那一答用它；其余答话是 0）
    pub a: u8,
    /// **那一条的身子**（只有 `embark` 那一答填它；其余答话是 [`TaskId::new(0)`]）
    pub task: TaskId,
}

impl Message for Said {
    /// **写法与读法是同一个**：这一形三格俱全，读的人不必再问"我问的是哪一条"
    type In = Said;
    /// 定长一答（Said::LEN）
    type Buf = [u8; Said::LEN];
    const EMPTY: Self::Buf = [0u8; Said::LEN];

    /// 要的是定长数组、返 `()`，两回事
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Said::store_at(self, out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Said> {
        if bytes.len() != Said::LEN {
            return None;
        }
        Said::fetch_at(bytes, 0).map(|one| one.0)
    }
}

/// 成功那一格：**全协议同一个号**——定义在 crate::wire::OK，本族只把它转出来
/// （crate::WireCodes 派生的两向读法就是拿它当"没失败"那一格）
pub use crate::wire::OK;

/// **一问的形状**——一条动作一格：荷载只有名字，"回信往哪"由 Ask::back 带
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Req {
    /// `MINT`：按名字起一条（建域 + 产代表线程，**恒产未放行**）
    Mint(String),
    /// `EMBARK`：放行 ＋ 等就绪
    Embark(String),
    Debark(String),
    Ruin(String),
    /// `STATE`：这一条此刻处于哪个阶段
    State(String),
}

impl Req {
    pub fn ask(self, back: PieToken) -> Ask {
        let (op, name) = match self {
            Req::Mint(name) => (MINT, name),
            Req::Embark(name) => (EMBARK, name),
            Req::Debark(name) => (DEBARK, name),
            Req::Ruin(name) => (RUIN, name),
            Req::State(name) => (STATE, name),
        };
        Ask { op, name, back }
    }
}

/// **收进来的一问**
/// 两格失败分得开：**长度不对** ⇒ 外层 `None`（连"往哪回"
/// 都没有 ⇒ 不动表、也不回话）；**动作码不认得** ⇒ 内层 `None`（这一问有回信的路，只是这一码
/// 我不认 ⇒ 回一句 BAD）
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wire {
    Mint(String),
    Embark(String),
    Debark(String),
    Ruin(String),
    State(String),
}

impl Wire {
    /// 解一问：`(读出来的动作, 回信孔那一格)`
    pub fn take(bytes: &[u8]) -> Option<(Option<Wire>, PieToken)> {
        // **"恰好"按游标判**：名字那一格是变长的，帧长不再等于 Ask::LEN（那是上界）。
        let (q, at) = Ask::fetch_at(bytes, 0)?;
        if at != bytes.len() {
            return None;
        }
        let ask = match q.op {
            MINT => Some(Wire::Mint(q.name)),
            EMBARK => Some(Wire::Embark(q.name)),
            DEBARK => Some(Wire::Debark(q.name)),
            RUIN => Some(Wire::Ruin(q.name)),
            STATE => Some(Wire::State(q.name)),
            _ => None,
        };
        Some((ask, q.back))
    }
}

/// 编一答：只有状态那一格（失败，或读不懂）
pub const fn said_status(status: u8) -> Said {
    Said {
        status,
        a: 0,
        task: TaskId::new(0),
    }
}

/// 编一答：`OK` ＋ 一个 State（只有 `state` 那一问用）
pub const fn said_state(state: State) -> Said {
    Said {
        status: OK,
        a: state.code(),
        task: TaskId::new(0),
    }
}

/// 编一答：`OK` ＋ **那一条的身子**（只有 `embark` 那一问用）
pub const fn said_task(task: TaskId) -> Said {
    Said {
        status: OK,
        a: 0,
        task,
    }
}

const _: () = assert!(ASK_MARK.get() != Mark::NONE.get());

const _: () = assert!(ASK_MARK.get() != BACK.get());

const _: () = assert!(ASK_MARK.get() != Mark::of(LINK).get());

const _: () = assert!(BACK.get() != Mark::NONE.get());

const _: () = assert!(BACK.get() != Mark::of(LINK).get());

// **答话那一形的宽度钉在编译期**：三格之和（状态 1 ＋ 答案 1 ＋ 身子那一格）。

const _: () = assert!(Said::LEN == 2 + <TaskId as env::wire::Field>::WIDTH);
