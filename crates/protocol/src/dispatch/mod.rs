//! dispatch — 服务目录协议（U/S 共享单一真相）。
//!
//! 四块分工（`docs/dispatch.md` 是规范，本模块是它的实现）：
//!   [`wire`]    —— 线格式：`Request`/`Reply`/`MSG_LEN`/`REPLY_AT`，**纯函数**（可宿主单测）；
//!   [`control`] —— 父域 ↔ 目录**控制孔**上的两条报文：`Refer`（引荐 / 预约）+ `Referred`；
//!   [`client`]  —— 客户端会话：`Directory`（连目录）+ `Service`（连服务）；
//!   [`server`]  —— 服务端：目录注册表 `Directory` + `serve()` 线格式适配。
//!
//! 路径保持不变：`protocol::dispatch::{MSG_LEN, Name, Reply, Request}` 照旧可用
//! （wire 出的东西在这里一并转出）——协议拆三块是**内部**整理，不动调用方的进口。
//!
//! 依赖方向：`protocol → runtime → env`。客户端与服务端都用 `runtime::env::mail`
//! 的门闩原语 + `runtime::core::port` 的一次往返；协议层**不碰** `env::ecall`。

pub mod client;
pub mod control;
pub mod server;
pub mod wire;

pub use client::{Directory, PAYLOAD_LEN, Service};
pub use control::{Refer, Referred};
pub use server::{DirectoryError, Release, Vestor, release_pie, vestor_of};
pub use wire::{
    CAP, E_DENIED, E_NOT_FOUND, E_TAKEN, Name, NameError, Op, ProtocolError, Query, Reply, TEXT,
};
