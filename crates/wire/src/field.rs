//! Byte-buffer codecs shared by protocol and environment frames.
//!
//! **`fetch` 的 `None` 说的是"这一帧读不懂"**，不是"这个字段的值不合规矩"——值那一层各有各的
//! 失败域（如名称的"装得下 / UTF-8"），故这里只答 `Option`。
//!
//! **帧**（`#[derive(Frame)]`，实现住 `mold`）的底座就是这里：**定长那一格**由 [`Field::WIDTH`]
//! 说宽度，**变长那一格**由 [`Span`] 的游标走出，从而两头不可能各写一份。

use alloc::string::String;

/// **过线的一格**：定宽 ＋ 会写会读。
pub trait Field: Sized {
    /// 线上占几字节（**定长**——帧的偏移全部由它求和得出）。
    const WIDTH: usize;
    /// 写进 `out`（长度恰是 [`Field::WIDTH`]）。
    fn store(&self, out: &mut [u8]);
    /// 从 `bytes` 读回来；**长度不足或那一格读不成** ⇒ `None`（不猜、不崩）。
    fn fetch(bytes: &[u8]) -> Option<Self>;
}

/// **要游标那一格**：宽度由值自己说（一条路几段、一段名字几字节），读写都从游标起。
///
/// 与 [`Field`] 的分工：[`Field`] 说"这一格多宽、怎么写"（**定长**——帧的偏移由它求和）；
/// 这一枚说"**最长**多少、从哪写起、写完到哪"。**定长那一格自动也是 `Span`**（下面那条 blanket
/// impl，`MAX = Some(WIDTH)`）——故一张字段表里两种格子可以并排，`#[derive(Frame)]` 只认 `Span`。
///
/// **（`MAX` 为什么是 `Option`）**：名字那一格（[`String`]）**不报上界**——它多长由**族**
/// 说（那一族的缓冲多大），不由类型说。于是"这一格最长几字节"变成一格**说得出来/说不出**的事实：
/// 说不出（`None`）的帧，`LEN` 就得由族显式给（`#[frame(len = …)]`），而不是各格求和。
pub trait Span: Sized {
    /// 这一格**最长**占几字节；**`None` = 这一格不报上界**（帧的界由族给）。
    const MAX: Option<usize>;
    /// 从 `at` 写起，返写完之后的游标。
    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize>;
    /// 从 `at` 读一格，返**值与读完之后的游标**。
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)>;
}

/// **定长那一格就是"不长的那一枚 `Span`"**：`MAX` 取 [`Field::WIDTH`]，两只手就是那两只。
///
/// 定宽字段共用同一套游标与边界检查。
impl<T: Field> Span for T {
    const MAX: Option<usize> = Some(<T as Field>::WIDTH);

    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let end = at.checked_add(<T as Field>::WIDTH)?;
        Field::store(self, out.get_mut(at..end)?);
        Some(end)
    }

    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let end = at.checked_add(<T as Field>::WIDTH)?;
        Some((Field::fetch(bytes.get(at..end)?)?, end))
    }
}

/// 各格上界求和；未知上界或算术溢出返回 None。
///
/// `const`：派生用它算 `LEN`（一处定义），故这两只手住在 `wire`，不散进宏里。
pub const fn total(parts: &[Option<usize>]) -> Option<usize> {
    let mut sum = 0usize;
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            Some(n) => match sum.checked_add(n) { Some(next) => sum = next, None => return None },
            None => return None,
        }
        i += 1;
    }
    Some(sum)
}

/// 按条数放大上界；未知上界或算术溢出返回 None。
pub const fn times(count: usize, one: Option<usize>) -> Option<usize> {
    match one {
        Some(one) => count.checked_mul(one),
        None => None,
    }
}

/// **一枚名（`String`）就是线上那一格**：`[长度那一字节][UTF-8 字节]`。
///
/// **（上界那一格为什么是 `None`）**：名字多长由**族**说（那一族的缓冲多大），不由这一枚
/// 类型说——故它不报上界，带它的帧要写 `#[frame(len = …)]`。真到落笔时有两个界、各管一头：
/// 长度那一字节（≤255，说得出多长）与那一帧的缓冲（族给的 `LEN`）。**任一条过不去 ⇒ `None`**
/// （不截断、不猜）。
///
/// **（它与「长度即内容」那一形的分工）**：这一手写的是**带长度**的形——名字在帧中间
/// （后面还有别的格）时非它不可；名字在**帧尾**时走 [`store_bytes`]／[`fetch_bytes`]（长度即内容，
/// 少一个字节）。
///
/// **（空是合法的值，不是错误）**：定长表要一个 `fill`，“空位”必须有值——那个值就是
/// `String::new()`。故这一格**不拒空**（拒空会让“合法类型的值”同时是“构造面拒绝的值”，
/// 又回到旧那一枚定宽名字的账上）。**“这一格有没有名字”是族读的时候用 `as_str().is_empty()` 判的**。
///
/// **（名字的长度不由这一层判）**：这一格的界只有两条，都在落笔那一刻：**长度那一字节
/// 放得下**（≤ 255）与**那一帧的缓冲装得下**（族给的 `LEN`）。故这里没有一枚“构造一枚名”的手——
/// 名就是 `String`，超界在编帧那一刻以“装不下”出现（用户裁定：界只由族那一处说）。
impl Span for String {
    /// **不报上界**：见上面那条。
    const MAX: Option<usize> = None;

    fn store_at(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let bytes = self.as_bytes();
        if bytes.len() > u8::MAX as usize {
            return None;
        }
        let end = at.checked_add(1 + bytes.len())?;
        let span = out.get_mut(at..end)?;
        span[0] = bytes.len() as u8;
        span[1..].copy_from_slice(bytes);
        Some(end)
    }

    fn fetch_at(bytes: &[u8], at: usize) -> Option<(String, usize)> {
        let len = *bytes.get(at)? as usize;
        let end = at.checked_add(1 + len)?;
        let text = core::str::from_utf8(bytes.get(at + 1..end)?).ok()?;
        Some((String::from(text), end))
    }
}

/// 单字节无符号整数。
impl Field for u8 {
    const WIDTH: usize = 1;
    fn store(&self, out: &mut [u8]) {
        out[0] = *self;
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        bytes.first().copied()
    }
}

/// 单字节布尔值；只接受 0 和 1。
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

/// **`u32` 那一格是 4 字节小端**：游标 / 线号 / 权的位数那几格就是它。
impl Field for u32 {
    const WIDTH: usize = 4;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_le_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
    }
}

/// 固定八字节数组，不解释内容。
impl Field for [u8; 8] {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(self);
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        bytes.get(..8)?.try_into().ok()
    }
}

/// 64 位无符号整数的小端表示。
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

// ── 尾巴：**变长那一段**（两种）──────────────────────────────
//
// 定长那一支由 `#[derive(Frame)]` 的字段表接手（`LEN` = 最长那一形之和，偏移一处都不写）；**变长**
// 那一支在本仓只有两种形状：
//
//   数得出来的   `[条数][条 × 一格]`   树那一族的 `seek` 路、它的「列」答、供单的名字表
//   长度即内容   `[…… 那些字节]`        树那一族的「名」答（名字多长，这一帧就多长）
//
// 下面两对就是这两处定义——**用户裁定：尾巴不许手写**（族里写 `2 + i * WIDTH` 或
// `out[1..1 + text.len()]` 这种句子，一条形状一处，四处就会漂）。
//
// 每一项的宽归它自己的 [`Span`] 说：`store_tail` / `fetch_tail` 里出现的每一个偏移都是**跑出来的
// 游标**，没有字面量——故这两只手**定长项与变长项同一条路**（定长项 `MAX` 就是它的 `WIDTH`）。

/// 从 `at` 起写下一段**同族的项**（定长那一形步长即它的宽），返写完之后的游标（一项都不写 ⇒ 原样返 `at`）。
///
/// `None` = `out` 装不下——与 [`Span::store_at`] 同一条口径（不猜、不截断）。
pub fn store_tail<T: Span>(out: &mut [u8], at: usize, items: &[T]) -> Option<usize> {
    let mut at = at;
    for item in items {
        at = item.store_at(out, at)?;
    }
    Some(at)
}

/// 从 `at` 起读一段**同族的项**，装进调用方给的容器；返读完之后的游标。
///
/// 容器由调用方提供，有几格位置就读几项；游标交回调用方，是否恰好到末尾
/// （如 `at == bytes.len()`）也由族自己判。
///
/// 短一字节、或某一格读不成 ⇒ `None`（与 [`Span::fetch_at`] 同一句话：这一帧读不懂）。
pub fn fetch_tail<T: Span>(bytes: &[u8], at: usize, into: &mut [T]) -> Option<usize> {
    let mut at = at;
    for slot in into.iter_mut() {
        let one = T::fetch_at(bytes, at)?;
        *slot = one.0;
        at = one.1;
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
/// 判**（如「名」那一答：先按 UTF-8 解，再进 the domain validator）。
pub fn fetch_bytes(bytes: &[u8], at: usize) -> Option<&[u8]> {
    bytes.get(at..)
}
