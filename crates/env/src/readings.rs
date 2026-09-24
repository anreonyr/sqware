//! 读数开关：**镜像里那些"给门看的行"要不要打**。
//!
//! # 为什么它是编译期常量，而不是运行期开关
//!
//! 打点只有两处源头：内核的 `console::_write` 与用户态的 `runtime::env::debug::put`。
//! 两处都要问同一个问题（"这一趟要不要读数"），而这个问题的答案**随镜像而不同**：
//! 验收/soak 那颗要（门按读数判），产品那颗不要（给出去的东西不该自说自话）。
//! 同一份源码 ⇒ 只能由构建期定；运行期问就得让内核知道"我这次被编进哪颗镜像"，
//! 而那件事内核本来不需要知道（装配单归 `env::assembly`）。
//!
//! 定法：`option_env!("SQWARE_READINGS")`。**默认不打**（未设置 = 静默）；要读数的那几门
//! 在编之前 `SQWARE_READINGS=1`（见 `crates/gate/src/lib.rs` 与 `crates/image`）。
//!
//! # 照实记（为什么不用 Cargo feature）
//!
//! 头一版想做 `#[cfg(feature = "readings")]`：不行。`env` 是**同一个 crate 同时供内核与
//! 用户态**（`libenv.rlib` 只编一份），而"内核那几行要留、程序那几十行要静"是**同一颗 ELF
//! 内部的分工**——feature 在 crate 内统一，做不到一半打一半不打。`option_env!` 同样是
//! 编译期定值，但它是**每个 crate 各自的**编译环境，`env` 编一份就带一个值，
//! 正好与"一个镜像一个值"对上。

/// 这一趟要不要读数。`false` = 静默镜像（产品那一颗）。
pub const READINGS: bool = option_env!("SQWARE_READINGS").is_some();
