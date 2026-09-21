//! driver — **驱动这一族**：设备面的持有者，与它们共用的装配件。
//!
//! 判据是**角色**，不是特权级（与 [`crate::stress`] 同款）：本目录下的成员各自在
//! `kernel/build.rs::INITRD_BINS` 声明自己是哪一档——今天两台都是 `Supervisor`，而
//! "驱动该 S 还是 U"这一格**还没有读数**（banner 里 UART 与 PLIC 的 PMP 都是 S/U (R,W)，
//! 故 U 态读得动设备；旧树的 `docs/driver.md §2` 裁过"驱动是 U 态域"）。
//!
//! ```text
//!   router   线路由者（中断面域）：持有中断控制器，接 / claim / complete 每一条线
//!   uart     串口驱动：持有 serial@10000000，把"收到字节就拉线"打开
//! ```
//!
//! **线的权威 / 属主 / 投递 / 收线**那一层要落在 `crates/protocol/src/driver`（正文与实现
//! 都还没写；`router` 今天只做"收与结"那一半）。
//!
//! # 两条家族纪律
//!
//! - **设备语义各带各的**：谁的设备谁在自己目录里放设备模块（[`router`] 的 `plic.rs`、
//!   [`uart`] 的 `uart.rs`）——本仓不用一份"驱动框架"去包它们。
//! - **需求单归收方**：[`router::needs`] / [`uart::needs`] 各开自己那张单，装配者只是
//!   `use` 它们（见 [`crate::supervisor::service::Program`]）。
//!
//! 本级的 [`assemble`] 是两台驱动**都要写一遍**的那一段客侧装配（会话 + 收配给 + 归位）。

pub mod assemble;
pub mod router;
pub mod uart;
