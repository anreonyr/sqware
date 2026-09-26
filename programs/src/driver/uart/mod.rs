//! uart::实现侧 — **串口驱动域**：`serial@10000000` 的持有者。
//!
//! `main.rs` 是它的入口（bin），`uart.rs` 是它的设备模块（也由那份 bin 自己 `mod` 声明
//! ——**同一份源码不编两遍**）。
//!
//! **照实记（本模块现在只有这几句定位）**：它原先还挂一格 `pub mod needs;`——只为把本域那张
//! 需求单交给 lib，好让 bin 经 `uart::needs::WANTS` 取到它；而那张单子只有**一行转发**
//! （定义在 [`plan::assembly::UART_WANTS`]）。这一刀删掉那一格，bin 直接从定义处取——与
//! `harness/src/lodger.rs`、`driver/rtc`、`driver/router` 同一条规矩。
//! 本模块留下是因为它是**这条路的锚**（`[`crate::driver::uart`]` 那类链接指着它）。
