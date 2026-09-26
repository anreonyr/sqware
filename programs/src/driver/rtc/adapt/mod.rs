//! rtc::adapt — **住持面（适配）**：碰内核、碰板、碰树、碰设备的那一半。
//!
//! ```text
//!   boot.rs      1–3：领配给归位 → 开图自证 → 上板（并开树那条会话）
//!   tree.rs      4  ：上树那一趟（`driver::tree::plate`）＋ 登记那一条线（`driver::register`）
//!   desk.rs      门面：解帧 → 认孔 → 喂会话核 → 执行它吐的答形
//!   resident.rs  5  ：常驻——一只组等两个源，喂事件、执行动作（**壳**）
//!   fail.rs      本域的死法（**下线**那一格）
//! ```
//!
//! **这一半由 bin 自己 `mod`**（不编进 lib）：它只属于这一台——设备模块（`rtc.rs`）同理。
//! 判定与状态在 `programs::driver::rtc::core`（纯）：这里的每一手要么是"取本核 → 转发"，
//! 要么是碰内核/设备的那一下。**死法住这里**（不在 `core/`）：它实现 `programs::Exit`、
//! 报的是内核出口那一行——那是程序侧的事；与 `core::fail`（**上线**那一格，讲客人那一问）
//! 是两件事，路径把它们分开了。

pub mod boot;
pub mod desk;
pub mod fail;
pub mod resident;
pub mod tree;
