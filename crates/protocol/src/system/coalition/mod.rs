//! system/coalition — **正文已搬进「约」**（`crates/contract/src/system/coalition/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造与两张对照表）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。


// ── 载体：三份各住哪里 ─────────────────────────────────────
//
// **判定与接口**（正文、六条原语、帧、客侧那一面）住在这里；**实现方**（真在
// `prog-coalition` 域里跑的那枚线程）住 `programs/src/system/coalition/`。
// 装配侧（谁在什么时候 `derive` + `bind`）住 `programs/src/service.rs`。

pub mod call;
pub mod client;
// 形与据已搬进「约」——转出。
pub use contract::system::coalition::{core, frame};

pub use call::{BACK, DIR, NAME};
pub use crate::system::coalition::core::{Coalition, CoalitionId, Fail, WINDOW_CAP, Window};
