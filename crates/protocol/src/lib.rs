#![no_std]
//! 一句话本身（正文 · 形 · 据 · 账）＋ 碰内核的那几件（开口）。
//! 一句话记：**协议是话，程序是说话的人**——域入口、那一台、装配表、需求单、死法、装配次序
//! 都住 `programs`。

// 配给那一段与几本账都是**可增长的**（Vec::try_reserve，备不下就如实报，不 panic）——与
extern crate alloc;

pub mod common;
/// `#[derive(crate::WireCodes)]`（驱动那一侧 `#[derive(protocol::WireCodes)]`）
pub use env::WireCodes;

pub mod communication;
pub mod debug;
pub mod driver;

pub mod service;
pub mod system;
pub mod wire;

/// **答话那一格的"没失败"**（0）——全协议**一个号**：那几族（principal / coalition / operator
/// / control / 设备账）与驱动各自那几族（如 programs::driver::rtc）共用
pub use wire::OK;

// 调试面那一支宏（`debug!`）住 `debug.rs`——**只在 debug 构建下有效**（见那个文件的头注）。
// 不必自己先有 `alloc` 这个前提（与上面 `OK` 的转出同一条规矩）。
#[doc(hidden)]
pub use alloc::format as __format;

// `identity-back` / `line-back`：同一张表里两面的回信孔若刻同一个记号，
// 就分不出这一枚是哪一面的。三对里 `principal ↔ coalition` 那一对钉在
// ——`frame.rs` 那两份只认得 `env` 与同层 `core`，看不见 `driver`。这一处看得见整棵树，故由它钉。
const _: () = assert!(
    crate::system::identity::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
