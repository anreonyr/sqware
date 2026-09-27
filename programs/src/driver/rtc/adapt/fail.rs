//! rtc::adapt::fail — 本域的死法：**一个号 ＋ 那句话**（一族口径在 [`driver::fail`]）。
//!
//! 本文件只留 rtc 自己的事实：**它在装配表上那一号**（`programs::program::rtc::E_RTC`
//! ——本域一个数都不写，见 `driver/fail.rs` 那条照实记），以及"配给那一趟没成"时那句话。
//! 一步一步的读数写在**死处**（`Fail::at(DIED, "rtc: desk")`）。
//!
//! **它与 `rtc::core::Fail` 是两件事**：这一枚是**下线**那一格（"这一域死在起手/常驻的哪一步"，
//! 读的人是内核出口与板那条死亡道）；那一枚是**上线**那一格（"客人那一问怎么了"，折成答码过线）。
//!
//! [`driver::fail`]: programs::driver::fail

use programs::program::{Died, rtc::E_RTC};

/// 本域在装配表上那一号（"rtc 死了"）。**本域一个数都不写**。
pub const DIED: Died = E_RTC;

/// "配给那一趟没成"时那句话（号由 `assemble` 原样带来）。
pub const ASSEMBLE: &str = "rtc: assemble";

/// 本域的死法（`main` 的返回类型）。
pub type Fail = programs::driver::fail::Fail;
