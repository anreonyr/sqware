//! 房间：一台域跑起来之后的那一面。
//!
//! 每核一张运行槽（`conductor`），跨核的放置与唤醒在 `scheduler`，没有活可跑之后
//! 各核走 `halt` 收场；等待站点与超时分别是 `messenger`、`deadline`。
//! 任务链（`chain`）与松弛计数（`counter`）是这一层共用的两件小东西。

mod chain;
pub(crate) mod conductor;
mod counter;
pub(crate) mod deadline;
pub(crate) mod halt;
pub(crate) mod identity;
pub(crate) mod interrupt;
pub mod messenger;
pub mod scheduler;
