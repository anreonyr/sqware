//! wire — **过线那一套**：定长一问一答的帧骨架（[`frame`]）· 一条报的约定（[`message`]）·
//! 协议的号（[`id`]）· 码表源（[`fail_codes`]）。

pub mod fail_codes;
pub mod frame;
pub mod id;
pub mod message;
