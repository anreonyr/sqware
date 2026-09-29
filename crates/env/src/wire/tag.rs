//! tag — **线上那一格定长名字**：32 字节、内容之后一个终止 NUL、其余全零。
//!
//! # 它是什么，不是什么
//!
//! **它不是字符串类型。** 本仓的串面照 std 原样住在 [`crate::text`]（[`CStr`] 借、
//! `CString` 拥有）；这一枚只管**线的形状**：这一位占多宽、内容怎么摆、填充怎么算。
//! 故它的读面**整块交给 [`CStr`]**（[`Deref`] 那一手——std 的 `CString: Deref<Target = CStr>`
//! 同形），本仓不再自造 `text`／`bytes`／`len` 那一套串接 API。
//!
//! **照实记（口径：上层不许回头改这一枚）**：`Copy`／`const`／定宽这三样只由**协议自己的事实**
//! 说（线上是定宽字段、一条路是定长坐标），**不由调用点说**。反过来也一样：这一枚的定宽不许
//! 去削 [`crate::text`] 那两枚（"给 `CString` 加内联定长 ＋ `Copy`"就是那么削出来的，故不做）。
//!
//! # 三条义务（构造面与解码面都判，不 panic、不截断）
//!
//! ```text
//!   非空      内容至少 1 字节——空块不算一条名字
//!   装得下    内容 < [`NAME_LEN`]（要留终止 NUL）
//!   无 NUL    内容里不许有 NUL（否则"内容到哪结束"有两说）
//!   UTF-8     只有线格式入口会遇到（[`Tag::new`] 收 `&str`，天然合法）
//! ```
//!
//! **照实记（这一刀把终止 NUL 从"通常有"改成"必有"）**：旧的解码面（`Name::from_bytes`）在
//! **整块 32 字节一个 NUL 都没有**时会放行（`bytes[len..]` 恰好为空 ⇒ 那一条"末端填充检查"
//! 是空转），于是能造出**没有终止符**的名字；而 [`Tag::from_slice`] 那一侧是拒的（`>= NAME_LEN`
//! ⇒ `None`）——**同一件事两份口径**。今天读面是 `&CStr`（unsized 视图，**必须**有终止 NUL），
//! `Deref` 要处处成立 ⇒ 三条入口收成**一条规矩**：没有终止 NUL 的块 ⇒ `None`（整帧按"读不懂"
//! 答 `BAD`）。**线上字节一个都没变**；变的是"那种病态的块"从"读成一条 32 字节的名字"改成
//! "读不懂"。全仓没有生产者会写出那种块（两条构造面都拒）。
//!
//! **照实记（内核不在这份名单里）**：域名字与线程名那一刀把内核里那两格删了（`Build` 不再收
//! 名字，身份只留号），认设备那一刀又把配对块里那格式设备 basename 换成了坐标（[`crate::Key`]）。
//! 故本格今天**只钉用户态协议**。
//!
//! **照实记（退掉的三笔账）**：
//!   · `Ord`／`PartialOrd`／`Hash`——旧注写着"供目录容器与排序枚举使用"，**假账**：全仓零
//!     `HashMap<Tag>`／零对它的 `sort*`（唯三处排序：`assemble.rs` 按 `order`、`machine.rs`
//!     按 `Key`、`frame.rs` 排页号）。
//!   · `NameError` 那一族——20 处引用**全在定义处自己身上**，调用方一律 `.ok()` 或
//!     `map_err(|_| …)` ⇒ 收成 [`Option`]（与 [`crate::wire::Field::fetch`] 同一条口径：
//!     `None` 说的是"这一格读不懂"）。
//!   · `len()`／`bytes()`／`text()`——`len()` 在旧文件之外**零读者**（要长度问
//!     [`CStr::count_bytes`]）；`bytes()` 全仓只有它自己的 [`Field`] impl 一处读者（并进那一手，
//!     见 [`Tag::block`]）；`text()` 只有一个真调用点 ⇒ 经 [`Deref`] 的 `to_bytes()`。

use core::fmt;
use core::ops::Deref;

use crate::text::CStr;

/// 名字字段字节数（含终止 NUL）——**单一真相**：板协议（`crates/protocol/src/system/board`）与
/// 树协议（`crates/protocol/src/system/operator`）共用同一上限（内容 ≤ 31 字节）。
pub const NAME_LEN: usize = 32;

/// 定长名字：32 字节、内容 ＋ 终止 NUL ＋ 零填充。
///
/// 类型义务：非法名不可表达——拿到 [`Tag`] 即已校验，调用方不再查。比较按整块定长字节
/// （填充由构造保证规范，故等值即语义等值）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tag {
    bytes: [u8; NAME_LEN],
}

impl Tag {
    /// 空的一格：**不是一条名字**，只是占位——定长表的空行要用一个可复制的初值。
    /// [`CStr::is_empty`]（经 [`Deref`]）认它，故它天然被排除在"有名字的行"之外。
    pub const EMPTY: Tag = Tag {
        bytes: [0u8; NAME_LEN],
    };

    /// 由字符串构造（非空、装得下、无 NUL；UTF-8 由 `&str` 免费）——非法 ⇒ `None`，不截断。
    ///
    /// **`const`**：树上的"路"（`protocol::system::operator::Path`）是**常量**——`/svc/sys/principal`
    /// 那种坐标要在 `const` 里造出来。代价是那两个容器方法（`contains` / `copy_from_slice`）不能
    /// 用（它们不是 `const`），改成手写循环：等价、无分配。
    pub const fn new(s: &str) -> Option<Tag> {
        let b = s.as_bytes();
        if b.is_empty() {
            return None;
        }
        if b.len() >= NAME_LEN {
            return None;
        }
        let mut bytes = [0u8; NAME_LEN];
        let mut i = 0;
        while i < b.len() {
            if b[i] == 0 {
                return None;
            }
            bytes[i] = b[i];
            i += 1;
        }
        Some(Tag { bytes })
    }

    /// 由线上**整块**还原（规范的填充 ＋ 内容合法）——[`Tag::new`] 的**线格式对偶**。
    ///
    /// 判据：找第一个 NUL；它就是终止符 ⇒ 它必须**在块内**（内容 < [`NAME_LEN`]）且其后全是零；
    /// 内容非空且是全法 UTF-8。任一条不成立 ⇒ `None`（这一帧读不懂）。
    pub fn from_block(bytes: [u8; NAME_LEN]) -> Option<Tag> {
        let len = bytes.iter().position(|&b| b == 0)?;
        if len == 0 {
            return None;
        }
        if bytes[len..].iter().any(|&b| b != 0) {
            return None;
        }
        if core::str::from_utf8(&bytes[..len]).is_err() {
            return None;
        }
        Some(Tag { bytes })
    }

    /// 由"**长度即内容**"那一段还原（帧尾那一格：内容多长，这一段就多长，**不带 NUL**）。
    ///
    /// 判据与 [`Tag::from_block`] 逐条相同，只是"终止 NUL 之后必须全零"这一条没有了——帧里
    /// 没有 NUL 之后。
    ///
    /// **`const`**（与 [`Tag::new`] 同一句照实记）：`Path::new` 在 `const` 里按 `/` 切段，
    /// 切出来的每一段都经这一手校验（`from_utf8` 自 1.63 起就是 `const`，故 UTF-8 那一问不动）。
    pub const fn from_slice(s: &[u8]) -> Option<Tag> {
        if s.is_empty() {
            return None;
        }
        if s.len() >= NAME_LEN {
            return None;
        }
        let mut bytes = [0u8; NAME_LEN];
        let mut i = 0;
        while i < s.len() {
            if s[i] == 0 {
                return None;
            }
            bytes[i] = s[i];
            i += 1;
        }
        match core::str::from_utf8(s) {
            Ok(_) => {}
            Err(_) => return None,
        }
        Some(Tag { bytes })
    }

    /// 本格的 [`CStr`] 视图——**std 的名字**（`CString::as_c_str` 同形），也是 [`Deref`] 的底座。
    pub fn as_c_str(&self) -> &CStr {
        match CStr::from_bytes_until_nul(&self.bytes) {
            Ok(c) => c,
            // 不变量：内容 < NAME_LEN ⇒ 块里必有终止 NUL（三条构造面都保证）
            Err(_) => unreachable!("tag block has no terminating NUL"),
        }
    }

    /// 内容（构造与解码都已校验 UTF-8 ⇒ **这一手不失败**）。
    ///
    /// 与 [`CStr::to_str`] 的分工：那一枚是 std 的（担不担保 UTF-8 是它的问题）；本格的线契约
    /// 里 UTF-8 是义务，故这里不必再兜一次底。
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.content()).unwrap_or("")
    }

    /// 那一块 32 字节（**ABI 那一面**：线上整格落字节要用它）。
    ///
    /// 为什么不是 [`CStr::to_bytes_with_nul`]：那一手只到终止 NUL 为止，而**线上这一格是定宽的**
    /// ——填充也要落上去（隐式尾巴就会把未初始化字节读上线）。它是本仓 ABI 的账，故只给本 crate。
    pub(crate) const fn block(&self) -> &[u8; NAME_LEN] {
        &self.bytes
    }

    /// 内容那几字节（终止 NUL 之前）。
    fn content(&self) -> &[u8] {
        let len = self.bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        &self.bytes[..len]
    }
}

/// **读面照 std 借**：本格是"一串 C 串字节"的定长持有者，故 [`Deref`] 到 [`CStr`]
/// （std 的 `CString` / `PathBuf` / `OsString` 都是这一手）。于是 `to_bytes` / `to_bytes_with_nul` /
/// `to_str` / `is_empty` / `count_bytes` 全部从 core 来，本仓一行不写。
impl Deref for Tag {
    type Target = CStr;

    fn deref(&self) -> &CStr {
        self.as_c_str()
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
