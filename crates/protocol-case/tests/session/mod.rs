//! `session` 这个名字的落点：**假手表 + 真据**。
//!
//! **照实记（这一台改过两次形状）**：从前 `core.rs` 写着 `use super::call`（直接叫内核那几手），
//! 于是这一台要**冒充整个 `call` 模块**——一份 224 行的影子桩。手表化（用户裁的甲）之后 `core`
//! 只要一张 [`Hands`](hands::Hands)：本台给一张**假手表**，形状由**编译器**检查。
//! **这就是那一刀的收益**：从前的影子桩连 `ship` 的返回类型都与真的不一样（真那份返
//! `Result<PieToken, ()>`、桩返 `Result<(), ()>`），照样编得过；现在这一格漂不了。
//!
//! **照实记（第二次：`#[path]` 退场）**：`hands` / `core` 原先各拿一行 `#[path]` 把
//! `contract/src/session/{hands,core}.rs` 逐字编进靶（那是「约」分家之前唯一的出路）。
//! `contract` 真依赖挂上之后那两行多余——**转出真那份**即可：同一份源码、同一个名字，
//! 一份都不多编，`call.rs` 里那两行 `use super::{core, hands}` 也一个字不改。

/// 本台的**假手表**（十个假手 ＋ 测试侧的观察面：`said` / `shipped` / `put` …）。
pub mod call;

/// 表的形状与会话的据——**真依赖** `contract` 那两份（逐字同一份源码）。
pub use contract::session::{core, hands};
