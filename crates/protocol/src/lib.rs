#![no_std]
//! protocol — **协议那一棵树**：一句话本身（正文 · 形 · 据 · 账）＋ 碰内核的那几件（开口）。
//! 一句话记：**协议是话，程序是说话的人**——域入口、那一台、装配表、需求单、死法、装配次序
//! 都住 `programs`。

// 配给那一段与几本账都是**可增长的**（`Vec::try_reserve`，备不下就如实报，不 panic）——与
// `env` / `runtime` 同款：这里引 `alloc`。
extern crate alloc;

pub mod common;
pub mod communication;
pub mod debug;
pub mod driver;




pub mod service;
pub mod system;
pub mod wire;

/// **答话那一格的"没失败"**（0）——全协议**一个号**：那几族（principal / coalition / operator
/// / control / 设备账）与驱动各自那几族（如 `programs::driver::rtc`）共用。
/// 定义在 [`fail_codes`] 那一份源里（`fail_codes!` 的第二个参数就是它）；这里把它**转出**
/// crate：`fail_codes` 那个模块自己是有意不进公共面的（出 crate 的只有那个宏），而驱动那一侧
pub use wire::fail_codes::OK;

// 调试面那一支宏（`debug!`）住 `debug.rs`——**只在 debug 构建下有效**（见那个文件的头注）。
// 它拿 `format!` 拼行，故把 `alloc` 那一支在这里转出：调用方（`programs` / `harness`）因此
// 不必自己先有 `alloc` 这个前提（与上面 `OK` 的转出同一条规矩）。
#[doc(hidden)]
pub use alloc::format as __format;

// `principal-back` / `coalition-back` / `line-back`：同一张表里两面的回信孔若刻同一个记号，
// 就分不出这一枚是哪一面的。三对里 `principal ↔ coalition` 那一对钉在
// `system::principal::frame`（那一对），**跨到 `driver::line` 的这两对钉在这里**
// ——`frame.rs` 那两份只认得 `env` 与同层 `core`，看不见 `driver`。这一处看得见整棵树，故由它钉。
const _: () = assert!(
    crate::service::principal::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
const _: () = assert!(
    crate::service::coalition::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
