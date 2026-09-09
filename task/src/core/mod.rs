//! core 适配层 —— 任务本地原语（heap/tls/unit）+ Mail 之上封装
//! （channel/service/directory/datagram/handshake）。
//!
//! 与 `env/` 的分工：`env/` 是 envcall 转发（薄），`core/` 是组合与封装（厚）。
//! 例子：`env::mail::HolePie` 是「薄」门闩句柄；`core::channel::Channel` 是
//! 「厚」开-关生命周期封装。

pub mod channel;
pub mod datagram;
pub mod directory;
pub mod handshake;
pub mod heap;
pub mod service;
pub mod tls;
pub mod unit;
