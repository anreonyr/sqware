//! supervisor — **这台机器的装配面**：设备需求单、boot 给的账、装配机器。
//!
//! 这三样都**不是某个 bin 的私物**：`needs` 是两端共用的契约、`pairing` 是三条路共用的编解码、
//! `service` 是"按单起服务"那一台机器。它们从前住在 `bin/supervisor/` 里，靠 `#[path]` 被
//! 七、七、两个 bin 各声明一遍（各自还得写"另一半是死码"）——那是**实现细节绑住了结构**。
//! 搬到 lib 之后各 bin 只 `use programs::supervisor::…`，一份源码编一次。
//!
//! **它们仍然是"这台机器的事实"**（设备名、启动参数、谁先起），故留在 `programs` 这一侧，
//! 不进 `packages/protocol`：协议面（判定 / 帧 / 账 / 三个角色）住那边。

pub mod boot;
pub mod needs;
pub mod pairing;
pub mod service;
