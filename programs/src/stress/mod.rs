//! stress — **压测台共用的那一件**：怎么量时间（刻度、校准、空转）。
//!
//! `churn` / `rig` / `busy` / `hang` / `load` 五者互不依赖，共享的只有这一份；它从前住在
//! `bin/stress/tick.rs`，由五个 bin 各 `#[path]` 声明一次（各带一句"另一半是死码"）。
//! 搬进 lib 之后各 bin 只 `use programs::stress::tick;`——与 [`crate::supervisor`] 同一条道理：
//! **共享面住 lib，一份源码编一次**。

pub mod tick;
