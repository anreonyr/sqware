//! rtc::实现侧 — **实时钟驱动域**：`rtc@101000` 的持有者。
//!
//! `main.rs` 是它的入口（bin），`rtc.rs` 是它的设备模块（也由那份 bin 自己 `mod` 声明
//! ——**同一份源码不编两遍**）。
//!
//! `call.rs` / `core.rs` / `client.rs` 由本模块收进 lib：后几份是**服务面**——驱动自己那份具体
//! 协议，**不进 `crates/protocol`**。客人（`harness/src/sleeper.rs`）与驱动 `use` 的是同一份源码。
//!
//! **照实记（`needs.rs` 那一格这一刀没了）**：它只有**一行转发**（定义在
//! [`plan::assembly::RTC_WANTS`]），bin 现在直接从定义处取——与 `driver/uart`、`driver/router`、
//! `harness/src/lodger.rs` 同一条规矩：一行转发不该撑起一个文件。

pub mod call;
pub mod client;
pub mod core;
