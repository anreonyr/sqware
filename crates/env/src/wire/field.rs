//! **按字节缓冲编解码**：过线那一格自己是多宽、怎么写进字节、怎么读回来。
//!
//! 与 [`Wire`](super::Wire) 的分工：`Wire` 是**六寄存器 ABI** 那一层（`[usize; 6]`），
//! 这一层是**报文帧**那一层（`&[u8]` / `&mut [u8]`）。两层都在 `env`：过线的那些东西住一处。
//!
//! **`fetch` 的 `None` 说的是"这一帧读不懂"**，不是"这个字段的值不合规矩"——值那一层各有各的
//! 失败域（如 [`NameError`](crate::wire::NameError)），故这里只答 `Option`。
//!
//! 本文件还有 `frame!` 宏（**定长帧**那一族的一处定义）：帧的偏移全部由 [`Field::WIDTH`]
//! 求和得出，从而两头不可能各写一份。

use crate::wire::{Name, PieToken, TaskId, NAME_LEN};

/// **过线的一格**：定宽 ＋ 会写会读。
pub trait Field: Sized {
    /// 线上占几字节（**定长**——帧的偏移全部由它求和得出）。
    const WIDTH: usize;
    /// 写进 `out`（长度恰是 [`Field::WIDTH`]）。
    fn store(&self, out: &mut [u8]);
    /// 从 `bytes` 读回来；**长度不足或那一格读不成** ⇒ `None`（不猜、不崩）。
    fn fetch(bytes: &[u8]) -> Option<Self>;
}

/// **`TaskId` 那一格是 8 字节小端**。
///
/// **照实记（这一对 impl 替掉了什么）**：系统那一层有**一条**"一个号过线"的帧（装配者告诉
/// 对面"以后答话的是这一位"），而它在**五处**各写了一遍——两处写
/// `(who.get() as u64).to_le_bytes()`（`programs/src/system/{board,operator}/bridge.rs` 的
/// `tell`），三处各按自己的读法现翻（两侧客人的 `hear` 与 `operator/server.rs` 的 `settle`：
/// `[0u8; 8]` ＋ `Ok(8)`、`get(..8)` ＋ `from_le_bytes`）。宽度与字节序写五遍 ⇒ 改一处漏一处
/// **编得过**，症状要等帧被读成"读不懂"才显形。故这一格**不另立一个只有一格字段的结构体**：
/// 它的形状就是一个 `TaskId`，一处定义在这里（那边几处的语义各不相同——"客人是谁"与
/// "答话的是谁"——一个名字反而会说错话）。
/// **一个裸字节也算一格**——动作码、条数那几格就是它（`WIDTH` = 1）。
///
/// **照实记（为什么不另立一个模型类型）**：报文头一格是"这是哪一条报"的动作码，它是帧布局的
/// 一部分（字段表的第一格），但它不属于任何一枚已有类型。而"动作码"那个名字树里已经有主
/// （37 处），故这里把 `u8` 本身认成一格：进出都是一个字节，码的**含义**仍归各族自己那枚
/// 私有常量说（表里只放值）。
impl Field for u8 {
    const WIDTH: usize = 1;
    fn store(&self, out: &mut [u8]) {
        out[0] = *self;
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        bytes.first().copied()
    }
}

impl Field for TaskId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&(self.get() as u64).to_le_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
        Some(TaskId::new(u64::from_le_bytes(raw) as usize))
    }
}

impl Field for PieToken {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        PieToken::from_bytes(bytes.get(..8)?)
    }
}

impl Field for Name {
    /// **定长、带填充**：`Name::bytes()` 是那 32 字节的整个数组（内容之后的填充也上线）。
    /// 帧的偏移要的是"这一格占多宽"，故取 `NAME_LEN`，不是内容的长度。
    const WIDTH: usize = NAME_LEN;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(self.bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Name::from_bytes(bytes.get(..NAME_LEN)?.try_into().ok()?).ok()
    }
}

// ── `frame!`：搬去 `envmacros` 了 ────────────────────────────
//
// 它从前就在这一格（`#[macro_export] macro_rules! frame`，故名字落在 **crate 根**上）。
// 改成**过程宏**（用户裁定）之后，实现住 `crates/envmacros/src/frame_impl.rs`，由 `env` 转出来
// （`crates/env/src/lib.rs` 的 `pub use envmacros::frame;`）——**调用点一个字没改**。
//
// 两件事因此变好：诊断指到**那一格字段**（`macro_rules` 只能报在展开体里）；名字不再在
// `env` 的 crate 根上当一条"与模块同名的宏"（第一刀与 `contract::frame` 撞的正是那一次）。
