#![no_std]
//! contract — **约**：两个 task 之间**那一句话本身**（形 · 据 · 账 · 不碰内核的适配）。
//!
//! # 划界只有一条判据
//!
//! > **本 crate 全树不碰 `runtime` 那一层。**
//!
//! 这条纪律从前写在**六份文件的头注**里（`system/board/core.rs`、`system/operator/core.rs`、
//! `session/core.rs`、`driver/line/core.rs`、`programs/.../board/desk.rs`、
//! `programs/.../operator/desk.rs`）——**有纪律，没有边界**。现在它是这个 crate 的存在理由：
//! 不碰内核的一切住这里，**宿主靶直接依赖本 crate**，不必再靠 `#[path]` 把源码逐字搬进测试。
//!
//! 碰内核的那一面住 `protocol`（**口**）：客侧那几手、会话那几手、落内核的适配。
//! 一句话记：**协议是话，程序是说话的人**——域入口、那一台、装配单、需求单、死法、装配次序
//! 都住 `programs`。
//!
//! # 面上有什么（分批搬进来；这里是**第一批**）
//!
//! ```text
//!   frame       形：principal 与 coalition **同形的那一份**骨架
//!   id          号：`Id` 那一族怎么编、怎么读
//!   fail_codes  负码表：`fail_codes!` 宏 ＋ 全协议共用的那一格 `OK`
//! ```
//!
//! **照实记（这三件为什么同批）**：`frame` 要 `id` 与 `fail_codes` ⇒ 三件是一组，只搬一件编不过。
//! 搬完 `protocol` 用 `pub use contract::{frame, id};` 与 `pub use contract::fail_codes::OK;`
//! **转出** ⇒ 调用点一处不改；宿主靶那几处 `#[path]` 改指新家。
//!
//! # 面会长成什么样（后面几批）
//!
//! ```text
//!   正文   `mod.rs`（薄：这句话是什么 ＋ 不变量）
//!   形     `frame.rs`
//!   据     `core.rs`
//!   账     `desk.rs`
//!   适配   `call.rs`（**不碰内核**的那几手：立板、注入、对照表）
//! ```
//!
//! **不进来**：`client.rs` 的客侧那几手、`session/call.rs` 的会话手——它们碰内核，住 `protocol`。
//! **照实记（一处按手切、不按文件切）**：`system/{board,operator}/call.rs` 现在不碰 `runtime`，
//! 但它 `pub use` 的三手（`marked_as` / `opened_by` / `vested_by`）由 `reserve_reads!` 包着
//! `mail::reserve`——**是内核读**。搬它们那一批时按手劈：立板与两张对照表进本 crate，
//! `ship` 与那三手进口。

// 配给那一段与几本账都是**可增长的**（`Vec::try_reserve`，备不下就如实报，不 panic）——与
// `protocol` 同款：这里引 `alloc`。
extern crate alloc;

pub mod fail_codes;
pub mod frame;
pub mod id;
pub mod system;
