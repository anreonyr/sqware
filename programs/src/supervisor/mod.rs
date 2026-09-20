//! supervisor — **S 态那一档**：装配面，以及只有监督侧用的那几片实现。
//!
//! 判据是特权级（唯一声明处：`kernel/build.rs::INITRD_BINS`）：本目录下都是 `Supervisor`。
//! 两样东西同住：
//!
//! - **装配面**（本级的 `boot` / `needs` / `pairing` / `service`）：设备需求单、boot 给的账、
//!   按单起服务那一台机器。它们都**不是某个程序的私物**：`needs` 是两端共用的契约、
//!   `pairing` 是三条路共用的编解码、`service` 是"按单起服务"那一台机器。它们从前住在
//!   `bin/supervisor/` 里，靠 `#[path]` 被七、七、两个程序各声明一遍（各自还得写"另一半是
//!   死码"）——那是**实现细节绑住了结构**。搬到 lib 之后各程序只 `use programs::supervisor::…`，
//!   一份源码编一次。它们仍是"这台机器的事实"（设备名、启动参数、谁先起），故留在 `programs`
//!   这一侧，不进 `crates/protocol`：协议面（判定 / 帧 / 账 / 三个角色）住那边。
//! - **只有监督侧用的实现**：[`board`]（板那一台，跑在装配域的宿主线程里）、[`firmware`]
//!   （引导域那圈发货循环）、[`system`]（编排域的实现，与它自己的 `main.rs` 同住）。
//!
//! 程序入口与它那片模块同住：`root/main.rs`、`plic/`、`system/main.rs` 都在本目录里。
//! U 态那一档在 [`crate::user`]；压测台**不分档**，整块留在 [`crate::stress`]。

pub mod board;
pub mod boot;
pub mod firmware;
pub mod needs;
pub mod pairing;
pub mod service;
pub mod system;
