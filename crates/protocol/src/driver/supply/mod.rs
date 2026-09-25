//! driver/supply — **正文已搬进「约」**（`crates/contract/src/driver/supply/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造与两张对照表）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。


// 形、据、客侧都已搬进「约」——这里**转出**（`crate::driver::supply::{frame,core,client}` 照旧解析）。
pub use contract::driver::supply::{client, core, frame};
pub use contract::driver::supply::frame::{BOOT, OP_SUPPLY, ORDER_CAP, REPLY_CAP, WANT_MAX};
pub use contract::driver::supply::core::Fail;
