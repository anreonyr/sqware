//! env 适配层 —— 每个调用域一个子模块，函数封 `*Call::X{..}.call()` 的域 Ret，
//! 零业务逻辑。厚封装（Channel/Directory/Service）见 `crate::core`。

pub mod chrono;
pub mod control;
pub mod io;
pub mod mail;
pub mod memory;
pub mod room;
pub mod task;
