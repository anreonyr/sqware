//! session::hands — **会话那一层的"手"**：碰内核的那十件，全部由「口」在构造时接上。
//!
//! # 为什么有一份这个
//!
//! 会话的据（[`core`](super::core)）要碰十处内核：铸孔、交出、放下、推、收、扫表、读两格事实、
//! 等、读钟。它们的**身体**在「口」那一侧（`protocol::session::call`），**这里只有形状**——
//! 十枚函数指针。于是：
//!
//! - 「约」自己**一件内核都不碰**（这是本 crate 的判据）。
//!
//! 甲（函数指针）是用户裁的；house 里已有同款的注入（`Board::new(vested_by, unship)`）。

use env::Wait;
use env::{Mark, PieToken, TaskId};

use super::core::Claim;

/// [`Each`] 交给回调的那一格：我表里的一枚孔 + **谁开的** + **刻的什么记号**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hole {
    pub token: PieToken,
    /// 这扇门谁开的（`None` = 查不出出处，如引导期那批设备门闩）。
    pub owner: Option<TaskId>,
    /// 这条路上刻的记号（[`Mark::NONE`] = 这一枚不是孔、问不到记号）。
    pub mark: Mark,
}

/// 往**对端**那一条泊位说一句话（等的那一版）。
pub type Post = fn(PieToken, &[u8]) -> Result<(), ()>;
/// 同上，槽满**当场**答 `Err`（不等）。
pub type TryPost = fn(PieToken, &[u8]) -> Result<(), ()>;
/// 从**本端**那一枚收一句话（有界等）。
pub type PullOwn = fn(PieToken, &mut [u8], Wait) -> Result<usize, ()>;
/// 铸一枚孔（记号刻在上面）。
pub type Unseal = fn(Mark) -> Result<PieToken, ()>;
/// 交出一枚副本（返"种在对端表里"的号）。
pub type Ship = fn(PieToken, TaskId) -> Result<PieToken, ()>;
/// 放下我这一份。
pub type Unship = fn(PieToken) -> Result<(), ()>;
/// 扫我这张表（不攒数组：枚数不设上限）。
pub type Each = fn(&mut dyn FnMut(Hole) -> Result<(), Claim>) -> Result<(), Claim>;
/// 这枚孔的两格事实：谁开的 / 刻的什么记号。
pub type Reserve = fn(PieToken) -> (Option<TaskId>, Mark);
/// 有界等（返 `false` = 没被叫醒）。
pub type Fall = fn(Wait) -> bool;
/// 单调钟（纳秒）。
pub type NowNs = fn() -> u64;

/// **十件手**，一张表。`Quay::open` 收它；每条泊位（`Pier`）只留下自己要的三枚。
#[derive(Clone, Copy)]
pub struct Hands {
    pub post: Post,
    pub try_post: TryPost,
    pub pull_own: PullOwn,
    pub unseal: Unseal,
    pub ship: Ship,
    pub unship: Unship,
    pub each: Each,
    pub reserve: Reserve,
    pub fall: Fall,
    pub now_ns: NowNs,
}
