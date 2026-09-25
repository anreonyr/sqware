//! driver/supply — **客侧在这一侧**（它要用船台）；形与据在「约」里。
//!
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。

pub mod client;

// 形、据**转出**（`crate::driver::supply::{frame,core}` 照旧解析）。
pub use contract::driver::supply::{core, frame};
pub use contract::driver::supply::frame::{BOOT, OP_SUPPLY, ORDER_CAP, REPLY_CAP, WANT_MAX};
pub use contract::driver::supply::core::Fail;
