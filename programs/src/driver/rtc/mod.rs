//! rtc::实现侧 — **实时钟驱动域**：`rtc@101000` 的持有者。
//!
//! `main.rs` 是它的入口（bin），`rtc.rs` 是它的设备模块（也由那份 bin 自己 `mod` 声明
//! ——**同一份源码不编两遍**）。
//!
//! `needs.rs` / `call.rs` / `core.rs` / `client.rs` 由本模块收进 lib：需求单是**收方自己开的**
//! （装配者照它开单），后三份是**服务面**——驱动自己那份具体协议，**不进 `crates/protocol`**。
//! 客人（`programs/src/user/sleeper.rs`）与驱动 `use` 的是同一份源码。

pub mod call;
pub mod client;
pub mod core;
pub mod needs;
