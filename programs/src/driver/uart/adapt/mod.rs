//! uart::adapt — **住持面（适配）**：只剩本域的死法。
//!
//! **照实记（`boot` / `tree` / `resident` 三份已退场）**：它们办的事——领配给、开图、上板、
//! 上树、占线、常驻那几件——三台驱动逐字同构，已按"同构才收"并进
//! [`programs::driver::{Device, Context}`]；本域的**主流程**因此回到 `main.rs` 那三段
//! （设备 / 入系统 / 核心）。
//!
//! **这一半由 bin 自己 `mod`**（不编进 lib）：本域的客人（`echo`）只经树拿到那枚孔，
//! **不读本域任何一份源码** ⇒ 连 `core/` 也在 bin 侧（与 `rtc` 那一侧相反，见 `driver/uart/mod.rs`）。
//! 死法住这里（不是 `uart/fail.rs`）：它实现 `programs::Exit`、报的是内核出口那一行——程序侧的事。

pub mod fail;
