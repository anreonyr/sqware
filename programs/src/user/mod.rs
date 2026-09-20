//! user — **U 态那一档**：不建域、不读设备、不碰 MMIO 的程序。
//!
//! 判据是特权级（唯一声明处：`kernel/build.rs::INITRD_BINS`）：本目录下都是 `User`。
//! [`operator`] 是那棵命名树的服务（一枚线程招待到底，`server.rs` 与它自己的 `main.rs` 同住）；
//! `echo.rs` / `guest.rs` / `passer.rs` 是三位客人，各是一份入口（不被 lib 收进来）。
//! 监督侧那一档在 [`crate::supervisor`]。

pub mod operator;
