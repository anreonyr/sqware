//! system/principal — **正文已搬进「约」**（`crates/contract/src/system/principal/mod.rs`）。
//!
//! 这里只剩**碰内核的那几件**（客侧那几手、十件手的身体、绑真手的构造）——
//! 判据见 `crates/protocol/src/lib.rs` 与 `crates/contract/src/lib.rs`。


// ── 载体：三份各住哪里 ─────────────────────────────────────
//
// **判定与接口**（正文、九条原语、帧、客侧那一面）住在这里；**实现方**（真在
// `prog-principal` 域里跑的那枚线程）住 `programs/src/system/principal/`。
// 装配侧（谁在什么时候 `derive` + `bind`）住 `programs/src/service.rs`。

pub mod client;
// 形与据已搬进「约」——转出。
pub use contract::system::principal::{core, frame};

pub use frame::{BACK, BAD, DIR, NAME, OK, Reply, Wire, code_to_fail, fail_to_code, reply_present};
pub use crate::session::call::opened_by;
pub use crate::system::principal::core::{Fail, Principal, PrincipalId};
