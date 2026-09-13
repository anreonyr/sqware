//! doom — 他杀（`kill`）的**用户态协议**：一条请求 + 一字节回执。
//!
//! # 机制在核，政策在服务
//!
//! 内核那枚 `RoomCall::Doom` 只认**血缘**（谁生的谁能杀，传递），而 root 是所有域的
//! 祖先 ⇒ 它对任何域的杀都够格。于是"**能不能**"由内核回答、"**该不该**"落在 root
//! 手里——这就是 Linux `kill` 的形状：谁都能请求，够格的那个来执行。内核因此不需要
//! 任何权限表，root 也不需要任何内核入口（`docs/root.md` §5.1）。
//!
//! # 三块分工（与 `dispatch`/`console` 同形）
//!
//!   [`wire`]   —— 线格式：`Kill`/`Ack`/`REQ_LEN`/`OP_*`，**纯函数、零依赖**；
//!   [`client`] —— 线对侧：`Doom`（连上服务、按名字杀）；
//!   [`server`] —— 服务侧：收一个名字（[`collect`]）+ 一条报文的结局（[`serve`]）。
//!
//! 服务线程本身住在 root 域（`programs/src/bin/supervisor/root`）：线程骨架与回执的
//! 推送是**装配**，"收谁、按什么判据收"才是协议——故 `serve` 不做 I/O，只回
//! [`Outcome`]。

pub mod client;
pub mod server;
pub mod wire;

pub use client::Doom;
pub use server::{GONE_ROUND_MS, GONE_ROUNDS, Outcome, collect, serve};
pub use wire::{ACK_LEN, Ack, Kill, OP_KILL, OP_QUIT, REQ_LEN, SERVICE};
