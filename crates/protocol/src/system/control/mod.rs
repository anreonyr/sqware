//! 系统里有什么 Service，它们处于什么生命状态。
//! ```text
//!   mint    造一个 Service（按名字：建域 + 产它的代表线程，恒产未放行）
//!            —— 镜像由**持表那一侧**从清单里取；**帧里不带镜像**
//!   embark   放行 + 等就绪（有通道的那条顺带逐条认领）

pub mod client;
pub mod frame;
pub mod marks;
pub mod grant;
pub mod publication;

pub use client::{BERTH, Face};
pub use frame::{ASK_MARK, BACK, DENIED, DIR, Fail, LINK, NAME, State};
pub use grant::{Grant, grant_of};
pub use publication::{Client, Frame, Object, Reply, Scope, Target};

pub mod account;
