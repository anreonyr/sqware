//! system::实现侧 — **编排域那一侧**：起一条、看/判、放下、收场。
//!
//! 系统协议的**规范与接口**（正文、判定、服务表、内核 ABI 的帧、子域怎么收配给）住
//! `crates/protocol/src/system/`；**实现方**（编排域怎么把一条服务起起来、怎么监督与收场）
//! 住这里。装配单与"这台机器有哪些服务"住在 [`crate::supervisor`]。

pub mod call;
pub mod server;
