//! user — **U 态那一档**：不建域、不读设备、不碰 MMIO，也不转授权。
//!
//! 判据是特权级（唯一声明处：`kernel/build.rs::INITRD_BINS`）：本目录下都是 `User`——
//! `echo.rs` / `guest.rs` / `passer.rs` 三位客人，各是一份入口（不被 lib 收进来）。
//!
//! 持树者（`operator`）**不在这里**：它 `ship` 带 `VEST` 的副本、是这台机器的转授权中枢，
//! 故与监督侧同档，住 [`crate::supervisor::operator`]。监督侧那一档整体在
//! [`crate::supervisor`]。
