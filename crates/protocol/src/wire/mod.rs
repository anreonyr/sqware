//! wire — **过线那一套**：定长一问一答的帧骨架（[`frame`]）· 一条报的约定（[`message`]）·
//! 协议的号（[`id`]）· 没失败那一格（[`ok`]）。

pub mod frame;
pub mod id;
pub mod message;
pub mod ok;

pub use ok::OK;
