//! uart::adapt::fail — 本域的死法：**一个号 ＋ 那句话**（一族口径在 [`driver::fail`]）。
//!
//! 本文件只留 uart 自己的事实：**它在装配表上那一号**（`programs::program::uart::E_UART`
//! ——本域一个数都不写，见 `driver/fail.rs` 那条照实记），以及"配给那一趟没成"时那句话。
//! 一步一步的读数写在**死处**（`Fail::at(DIED, "uart: tree")`）。
//!
//! [`driver::fail`]: programs::driver::fail

use programs::program::{Died, uart::E_UART};

/// 本域在装配表上那一号（"uart 死了"）。**本域一个数都不写**。
pub const DIED: Died = E_UART;

/// "配给那一趟没成"时那句话（号由 `assemble` 原样带来）。
pub const ASSEMBLE: &str = "uart: assemble";

/// 本域的死法（`main` 的返回类型）。
pub type Fail = programs::driver::fail::Fail;
