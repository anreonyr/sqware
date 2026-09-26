//! **按字节缓冲编解码**：过线那一格自己是多宽、怎么写进字节、怎么读回来。
//!
//! 与 [`Wire`](super::Wire) 的分工：`Wire` 是**六寄存器 ABI** 那一层（`[usize; 6]`），
//! 这一层是**报文帧**那一层（`&[u8]` / `&mut [u8]`）。两层都在 `env`：过线的那些东西住一处。
//!
//! **`fetch` 的 `None` 说的是"这一帧读不懂"**，不是"这个字段的值不合规矩"——值那一层各有各的
//! 失败域（如 [`NameError`](crate::wire::NameError)），故这里只答 `Option`。
//!
//! **定长帧**（`#[derive(Frame)]`，实现住 `mold`）的底座就是这里：帧的偏移全部由
//! [`Field::WIDTH`] 求和得出，从而两头不可能各写一份。

use crate::wire::{NAME_LEN, Name, PieToken, TaskId};

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

/// **`bool` 那一格是 1 字节，而且只许 `0` / `1`**：写出去只写这两格，读回来**别的字节一律
/// 读不懂**。
///
/// **照实记（它为什么是一格）**：树那一族 `land` 的"**改**那一轴"在模型里就是 `bool`
/// （`mine`），而它在线上占一格。族里把它写成 `u8` 再现翻，就把"哪一格是布尔"这条事实挪进了
/// 族的正文——与上面 [`u8`] 那一格同一条口径：**进出都是这一格**，含义留给族自己那枚私有常量。
///
/// **照实记（它原来不是这样）**：原先写的是"非零即真"（`*bytes.first()? != 0`）——`2` 被读成
/// "真"。而同 crate 的 `Wire for bool`（`wire/mod.rs`）一直是严格那一条
/// （`0 => false / 1 => true / _ => Err(Invalid)`）⇒ 同一件事两份口径，一处宽一处紧。今天按
/// `Wire` 那一份收齐：**这一处是编码面唯一的口径**，各族不再自己写 0/1 的判。
///
/// **照实记（coalition 那格旧的辩解已失效）**：盟籍的「未完」那一格（`SeqHead.more`）从前是
/// 裸 `u8`，理由正是"这里的读法太宽"；它今天改回真 `bool`，那句手写的 0/1 判一并退场。
impl Field for bool {
    const WIDTH: usize = 1;
    fn store(&self, out: &mut [u8]) {
        out[0] = *self as u8;
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        match bytes.first()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

/// **8 个裸字节也算一格**：写什么读什么，**含义归族说**（树那一族答话里"那一格号"就是它
/// ——`part` / `seek` 读成**坐标**、`find` 读成**门闩**，线上逐字同形）。
///
/// **照实记（为什么不拿 `PieToken` / `EntryId` 当这一格的类型）**：那是**两个号空间**，而这一格
/// 装不下"是哪一种"这条信息；拿其中一枚当类型，就把"问的是哪一条"写进了字段表——而它由
/// **问的人**认（他自己知道）。与 [`u8`] 那一格同一条口径：`Field` 只管这一格多宽、怎么落字节。
impl Field for [u8; 8] {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(self);
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        bytes.get(..8)?.try_into().ok()
    }
}

/// **`u64` 那一格是 8 字节小端**：名册/盟册那一族一问里的 `a` / `b` 两格就是它（那里的意义
/// 由**动作码**定，故字段表只管这 8 个字节怎么落）。
///
/// **照实记（它与 [`TaskId`] / [`PieToken`] 那两个 8 字节的区别）**：那两枚是**过线的号**
/// （各有校验：令牌自 1 起、`TaskId` 有 0 哨兵），而这一格是**裸的 8 字节数**——它的解释
/// （是身份号、是盟号、还是游标）归族的正文说，故这里只写宽度与字节序。
impl Field for u64 {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let raw: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
        Some(u64::from_le_bytes(raw))
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

// ── 尾巴：**变长那一段**（两种）──────────────────────────────
//
// 定长那一支由 `#[derive(Frame)]` 的字段表接手（`LEN` = 宽度之和，偏移一处都不写）；**变长**那一支在
// 本仓只有两种形状：
//
//   数得出来的   `[条数][条 × 等宽项]`   树那一族的 `seek` 路、它的「列」答，供单的 `n × 32`
//   长度即内容   `[…… 那些字节]`          树那一族的「名」答（名字多长，这一帧就多长）
//
// 下面两对就是这两处定义——**用户裁定：尾巴不许手写**（族里写 `2 + i * WIDTH` 或
// `out[1..1 + text.len()]` 这种句子，一条形状一处，四处就会漂）。
//
// 宽度仍归 [`Field::WIDTH`] 说：`store_tail` / `fetch_tail` 里出现的每一个偏移都是**跑出来的
// 游标**，没有字面量。

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

/// 从 `at` 起写下一段**裸字节尾巴**（**长度即内容**：没有条数、也没有终止符），返写完的游标。
pub fn store_bytes(out: &mut [u8], at: usize, bytes: &[u8]) -> Option<usize> {
    let end = at.checked_add(bytes.len())?;
    out.get_mut(at..end)?.copy_from_slice(bytes);
    Some(end)
}

/// 从 `at` 起读**到末尾**那一段裸字节（`None` = 起点越界）。
///
/// 长度即内容 ⇒ 读的人拿到的就是"这一帧还剩下的那些字节"；**这些字节算不算一段合法的内容由族
/// 判**（如 `Name::from_slice` 那四格）。
pub fn fetch_bytes(bytes: &[u8], at: usize) -> Option<&[u8]> {
    bytes.get(at..)
}

// ── `Frame`：搬去 `mold` 了 ────────────────────────────
//
// 它从前就在这一格（`#[macro_export] macro_rules! frame`，故名字落在 **crate 根**上）。
// 用户裁定先改成过程宏，又收成 `#[derive(Frame)]`：实现住 `crates/mold/src/frame.rs`，
// 由 `env` 转出来（`crates/env/src/lib.rs` 的 `pub use mold::Frame;`）。
//
// 三件事因此变好：诊断指到**那一格字段**（`macro_rules` 只能报在展开体里）；名字不再是
// `env` 的 crate 根上一条"与模块同名的宏"（第一刀与 `contract::frame` 撞的正是那一次）；
// 结构体现在**写在调用点**——各格的 `pub` 与字段上的文档都在用户那一边看得见，而生成的
// `LEN` / `store` / `store_in` / `fetch` 一个字没变（展开物逐字节比对过）。
