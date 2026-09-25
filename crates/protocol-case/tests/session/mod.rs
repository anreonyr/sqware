//! `session` 这个名字的落点：**假手表 + 逐字未改的据**。
//!
//! **照实记（这一台改过一次形状）**：从前 `core.rs` 写着 `use super::call`（直接叫内核那几手），
//! 于是这一台要**冒充整个 `call` 模块**——一份 224 行的影子桩。手表化（用户裁的甲）之后 `core`
//! 只要一张 [`Hands`](hands::Hands)：本台给一张**假手表**，形状由**编译器**检查。
//! **这就是那一刀的收益**：从前的影子桩连 `ship` 的返回类型都与真的不一样（真那份返
//! `Result<PieToken, ()>`、桩返 `Result<(), ()>`），照样编得过；现在这一格漂不了。

/// 本台的**假手表**（十个假手 ＋ 测试侧的观察面：`said` / `shipped` / `put` …）。
pub mod call;

/// 表的形状——**与真那份同一份源码，逐字未改**（`contract/src/session/hands.rs`）。
#[path = "../../../contract/src/session/hands.rs"]
pub mod hands;

/// 会话的据（就是 `crates/contract/src/session/core.rs` 那一份，逐字未改）。
#[path = "../../../contract/src/session/core.rs"]
pub mod core;
