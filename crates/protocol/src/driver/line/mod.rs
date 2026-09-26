//! driver/line — **正文已搬进「约」**（`crates/contract/src/driver/line/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造与两张对照表）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。

pub mod client;

// 据与形已搬进「约」——这里**转出**（`crate::driver::line::frame` 照旧解析）；
// **客侧留在本侧**：它自己铸孔、自己 `claim`，碰内核（与 `supply::client` 的判据正好相反——
// 那一份吃一枚注入的 `&Pier`，故进了「约」）。
pub use crate::driver::line::core::{Fail, Lines};
pub use contract::driver::line::{core, frame};
