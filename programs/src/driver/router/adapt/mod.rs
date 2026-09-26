//! router::adapt — **住持面（适配）**：碰内核、碰板、碰树、碰设备的那一半。
//!
//! ```text
//!   boot.rs      起手：领配给 → 开两图 → 读树 → 建账 → 铸入口 → 上板 ＋ 上树 → 挂组
//!   desk.rs      门面上那一句话：登记（解帧 → 解树 → 占格 → 接线 → 答）
//!   sweep.rs     逐客：主人没了的那些线——拆线 + 空出格子
//!   exhaust.rs   排空：客人说"我排空了"——那一格回闲 + 把线放回去
//!   bell.rs      铃：领一条 → 投一帧 → 投到了才静音 ＋ 结 → 报过没有那一行
//!   resident.rs  常驻**壳**：等三源 → 四手各就位
//!   fail.rs      本域的死法（**下线**那一格）
//! ```
//!
//! 判定不在这里：账与四原语住 `contract::driver::line::core`，"区 ↔ 线号"住
//! `crate::core::sources`（两者都是纯的）。本层只做"等、取、喂、执行"与碰硬件的那几下；
//! **死法住这里**（不在 `core/`）：它实现 `programs::Exit`，是程序侧的事。

pub mod bell;
pub mod boot;
pub mod desk;
pub mod exhaust;
pub mod fail;
pub mod resident;
pub mod sweep;
