//! :frame 的词汇那一半：状态（State）· 失败词汇（Fail）与两向换算 · 四手码 ·
//! 状态码 · 记号与那一段路（`LINK`/`NAME`/`ASK_MARK`/`BACK`/`DIR`）。


use crate::common::path::Path;
use crate::wire::OK; // `WireCodes` 派生的两向读法要用它（本文件是枚举的家）

/// 五格与 `programs/src/system/common/face/desk.rs` 的 `State` 逐格对应，且**只描述实例的生命阶段**
/// `crates/protocol/src/system/mod.rs` 的"预算与放弃"）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// 表里有这一行，但还没起过
    NeverStarted,
    /// 起了，正在等它宣布就绪
    Starting,
    /// 已就绪
    Ready,
    /// 已下令收，还没确认收干净
    Stopping,
    Dead,
    Debarked,
}

impl State {
    /// 线上那一格数（**判别值**：`State` 的码与内核无关，是本协议自己的）
    pub const fn code(self) -> u8 {
        match self {
            State::NeverStarted => 0,
            State::Starting => 1,
            State::Ready => 2,
            State::Stopping => 3,
            State::Dead => 4,
            State::Debarked => 5,
        }
    }

    /// 那一格数 → 状态。表外的数 ⇒ `None`（**不猜**）
    pub const fn of_code(code: u8) -> Option<State> {
        match code {
            0 => Some(State::NeverStarted),
            1 => Some(State::Starting),
            2 => Some(State::Ready),
            3 => Some(State::Stopping),
            4 => Some(State::Dead),
            5 => Some(State::Debarked),
            _ => None,
        }
    }
}

/// 失败域：五格，**前四格各对应一个不同的下一步**（照实抄 `programs/src/system/core.rs` 那四格）
/// 它是**协议这一侧**的名字：调度侧那四格是 `Unknown` / `BadImage` / `Full` / `NotReady`
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::WireCodes)]
#[wire(fallback = Bad)]
pub enum Fail {
    /// 表里没这个名字，或它已经登记过
    #[code(1)]
    Unknown,
    /// 镜像装不上（内核 UnitFail::BadImage）
    #[code(2)]
    BadImage,
    /// 表满，或线程 / 帧产不出来（内核 UnitFail::OoM）
    #[code(3)]
    Full,
    /// 没就绪：等到期还没起来、半路死了、或此刻不该起（已在跑）
    #[code(4)]
    NotReady,
    /// **本端读不懂那一句**（帧坏了 / 答话那一格解不动 / 期限到了还没答）
    /// 它在**失败表外**（同板、树那两族的先例）：它不是"持表那一侧说的事"，是**这一问没走到**
    /// 对本端而言与"这条路别指望了"同一个下一步，故不往 Fail 的语义格里塞
    #[code(5)]
    Bad,
    #[code(6)]
    Denied,
}

pub const MINT: u8 = 1;

pub const EMBARK: u8 = 2;

pub const DEBARK: u8 = 3;

pub const STATE: u8 = 4;

pub const RUIN: u8 = 5;

/// 这条路叫什么（泊位那一格）：**两侧同一个**
pub use crate::system::control::marks::LINK;

/// 这一面在树上的名字（挂到 `/svc/sys/control`）：**与 LINK 同一个串**——"泊位叫 `control`"
/// 与"它挂在哪一格"是同一件事的两层，重名不是重名
pub const NAME: &str = "control";

/// 问话孔那一枚上的记号。**带面名**（`control-ask`）：认领键是"谁开的 + 记号"，而同一枚任务
/// 可能同时是两面的客人——两枚孔都铸在它自己那张表里，记号再一样就分不开（理由与实测见
pub use crate::system::control::marks::ASK_MARK;

pub use crate::system::control::marks::BACK;

/// **本族那块窗格在树上的路**：`/svc/sys/control`（头两段是四族共用的
/// coalition）
pub const DIR: &Path = Path::new("svc/sys/control");

pub const INSTANCE_EMBARK: u8 = 6;
pub const INSTANCE_DEBARK: u8 = 7;
pub const INSTANCE_RUIN: u8 = 8;
pub const INSTANCE_STATE: u8 = 9;
