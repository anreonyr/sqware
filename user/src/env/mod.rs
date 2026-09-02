//! env 适配层 —— 每个 Ucall 域一个子模块，函数转发 `UcallBuilder`，零业务逻辑。

pub mod chrono;
pub mod control;
pub mod io;
pub mod mail;
pub mod memory;
pub mod room;
pub mod task;
