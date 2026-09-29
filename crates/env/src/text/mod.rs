//! text — **本仓的串面：照 std 原样转出**（这一层不受上层影响，也不给它加料）。
//!
//! ```text
//!   借（unsized 视图，NUL 终止）  core::ffi::CStr       from_bytes_until_nul / to_bytes /
//!                                                     to_bytes_with_nul / to_str / is_empty /
//!                                                     count_bytes   —— 后四条是 const fn
//!   拥有（堆、非 Copy、可变长）    alloc::ffi::CString  new / as_c_str / into_bytes_with_nul …
//! ```
//!
//! **两枚都是转出，一个字都不新写**（`pub use` 就是全部内容）。
//!
//! # 照实记（口径：上层不许回头改这一层）
//!
//! 上层的形状需求——**定宽、`Copy`、`const`、无生命周期**——不许回头改这两枚。那是
//! **线上那一格**（[`crate::wire::Tag`]：32 字节、NUL 终止、其余零填充）自己的事：它照
//! std 的 `CString: Deref<Target = CStr>` 那一手把读面整块交给 [`CStr`]，本仓不再自造
//! `text`／`bytes`／`len` 那一套串接 API。
//!
//! **照实记（为什么不做"内联定长 ＋ `Copy` 的 `CString`"）**：那样两处都会偏离 std 那一枚
//! （std 的 `CString` 是 `Vec<u8>` 堆形，也不是 `Copy`），而偏离的**唯一的理由**是"上层那 9 处
//! `const`、9 处定长字段、`Req`/`Wire`/`Tip` 的 `Copy` 装不下堆形"——**那就是上层影响下层**。
//! 故不做：定宽那一格归 `Tag`，堆形那一枚归 `CString`，两件事不互相削。
//!
//! **照实记（`CString` 今天零客人）**：全仓今天没有一处要"运行期拥有且可变长的线串"
//! （`programs` 代码面 `String` 0 处、`CString` 0 处；那两处 `format!` 是转瞬即弃的），
//! 故它**立词不立用**——等第一个客人。立词不是洁癖：它是这一层与 std 的接口面，客人哪天
//! 来（拼名字、攒一段文本），形状已经在这儿。
//!
//! **照实记（`CStr::len` 已改名）**：本机 nightly 上 `CStr` **没有** `len`，改叫 `count_bytes`
//! （另有 `bytes`）；照它写就得跟着走——这正是"搬 std 的东西"的日常账。

pub use alloc::ffi::CString;
pub use core::ffi::CStr;
