//! router::实现侧 — **线路由者（中断面域）**：程序本体、设备面，与它自己那片硬件账。
//!
//! `main.rs` 是它的入口（bin），`plic.rs` 是它的设备模块（也由那份 bin 自己 `mod` 声明
//! ——**同一份源码不编两遍**）。
//!
//! **照实记（本模块现在只有这几句定位）**：它原先还挂一格 `pub mod needs;`——那一格装两样：
//! 一行转发（[`plan::assembly::ROUTER_WANTS`]）与一个 `PLIC` 常量。前者删掉、bin 直接从定义处取；
//! 后者归到**定义处**（[`plan::assembly::PLIC_CLASS`]）——它原先是同一串字面量的**第二份**，
//! 而 `plic.rs` 那句"与单子上那一格是同一个常量"当时并不成立。与 `harness/src/lodger.rs`、
//! `driver/uart`、`driver/rtc` 同一条规矩：**一行转发不该撑起一个文件**。
//! 本模块留下是因为它是**这条路的锚**（`[`crate::driver::router`]` 那类链接指着它）。
