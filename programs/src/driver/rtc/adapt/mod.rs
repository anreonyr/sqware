//! rtc::adapt — **住持面（适配）**：碰内核、碰板、碰设备的那一半。
//!
//! ```text
//!   desk.rs      门面：解帧 → 认孔 → 喂会话核 → 执行它吐的答形
//!   resident.rs  常驻：一只组等两个源，喂事件、执行动作（**壳**）
//! ```
//!
//! **照实记（`boot.rs` / `tree.rs` 已退场）**：领配给、开图、上板、上树、占线那几步三台逐字
//! 同构，已并进 [`programs::driver::{Device, Context}`]；本域的**主流程**回到 `main.rs` 三段。
//! **留下的两份是 rtc 自己的形状**：门面（两个方向的服务面）与"等两个源"那一只组。
//!
//! **照实记（`fail.rs` 也退场了）**：本域那份薄壳（`DIED` / `ASSEMBLE` / `type Fail`）在残枝
//! 第二刀删掉——本域现在直接用 [`programs::program::rtc::E_RTC`] 与
//! [`programs::driver::fail::Fail`]，见那份的文件头。
//!
//! **这一半由 bin 自己 `mod`**（不编进 lib）：它只属于这一台——设备模块（`rtc.rs`）同理。
//! 判定与状态在 `programs::driver::rtc::core`（纯）：这里的每一手要么是"取本核 → 转发"，
//! 要么是碰内核/设备的那一下。**死法实现 `programs::Exit`、报的是内核出口那一行**——那是
//! 程序侧的事；与 `core::fail`（**上线**那一格，讲客人那一问）是两件事，路径把它们分开了。

pub mod desk;
pub mod resident;
