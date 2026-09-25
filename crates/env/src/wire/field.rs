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

/// **`bool` 那一格是 1 字节**：写出去只写 `0` / `1`，读回来**非零即真**（老帧里那一格是任意
/// 字节也照旧读得出"真"）。
///
/// **照实记（它为什么是一格）**：树那一族 `land` 的"**改**那一轴"在模型里就是 `bool`
/// （`mine`），而它在线上占一格。族里把它写成 `u8` 再现翻，就把"哪一格是布尔"这条事实挪进了
/// 族的正文——与上面 [`u8`] 那一格同一条口径：**进出都是这一格**，含义留给族自己那枚私有常量。
impl Field for bool {
    const WIDTH: usize = 1;
    fn store(&self, out: &mut [u8]) {
        out[0] = *self as u8;
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(*bytes.first()? != 0)
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

// ── 尾巴：**数得出来的一段** ────────────────────────────────
//
// 定长那一支由 `frame!` 的字段表接手（`LEN` = 宽度之和，偏移一处都不写）；**变长**那一支在
// 本仓只有一种形状：**一格条数 ＋ 那么多个等宽项**（树那一族的 `seek` 路、它的"列"答，供单的
// `n × 32`，盟籍那一扇窗）。下面这一对就是"那么多个等宽项"那一处定义——**用户裁定：尾巴
// 不许手写**（族里写 `2 + i * WIDTH` 这种句子，一条形状一处，四处就会漂）。
//
// 宽度仍归 [`Field::WIDTH`] 说：这一对里出现的每一个偏移都是**跑出来的游标**，没有字面量。

/// 从 `at` 起写下一段**等宽项**，返写完之后的游标（一项都不写 ⇒ 原样返 `at`）。
///
/// `None` = `out` 装不下——与 [`Field::store`] 那一族同一条口径（不猜、不截断）。
pub fn store_tail<T: Field>(out: &mut [u8], at: usize, items: &[T]) -> Option<usize> {
    let mut at = at;
    for item in items {
        item.store(out.get_mut(at..at + T::WIDTH)?);
        at += T::WIDTH;
    }
    Some(at)
}

/// 从 `at` 起读一段**等宽项**，装进调用方给的容器；返读完之后的游标。
///
/// **容器由调用方给**（`&mut [Name]` / `&mut [EntryId]` / …）：`env` 不认识 `alloc`，也不替族
/// 决定"装不下时丢哪一头"——**有几格位置就读几条**，游标交回去，于是"到这儿就是底"
/// （如 `at == bytes.len()`）也由族自己判。
///
/// 短一字节、或某一格读不成 ⇒ `None`（与 [`Field::fetch`] 同一句话：这一帧读不懂）。
pub fn fetch_tail<T: Field>(bytes: &[u8], at: usize, into: &mut [T]) -> Option<usize> {
    let mut at = at;
    for slot in into.iter_mut() {
        *slot = T::fetch(bytes.get(at..at + T::WIDTH)?)?;
        at += T::WIDTH;
    }
    Some(at)
}

// ── `frame!`：搬去 `envmacros` 了 ────────────────────────────
//
// 它从前就在这一格（`#[macro_export] macro_rules! frame`，故名字落在 **crate 根**上）。
// 改成**过程宏**（用户裁定）之后，实现住 `crates/envmacros/src/frame_impl.rs`，由 `env` 转出来
// （`crates/env/src/lib.rs` 的 `pub use envmacros::frame;`）——**调用点一个字没改**。
//
// 两件事因此变好：诊断指到**那一格字段**（`macro_rules` 只能报在展开体里）；名字不再在
// `env` 的 crate 根上当一条"与模块同名的宏"（第一刀与 `contract::frame` 撞的正是那一次）。
