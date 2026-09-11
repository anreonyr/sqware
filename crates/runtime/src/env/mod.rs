//! env 适配层 —— 每个调用域一个子模块，函数封 `*Call::X{..}.call()` 的域 Ret，
//! 零业务逻辑。厚封装（Channel/Directory/Service）见 `crate::core`。
//!
//! **原 `io` 子模块（`IOCall::Put`/`Get` 的转发）已删**：设备不在内核手里之后，
//! "往哪里写、从哪里读"是持设备者的事——控制台服务持 UART 门闩自己读写，客户端
//! 走 `protocol::console` 的会话。故这一层不再有 IO 面（`docs/driver.md` §10）。

pub mod chrono;
pub mod control;
pub mod mail;
pub mod memory;
pub mod room;
pub mod task;
