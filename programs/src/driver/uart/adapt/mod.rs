//! uart::adapt — **住持面（适配）**：碰内核、碰板、碰树、碰设备的那一半。
//!
//! ```text
//!   boot.rs      1–3：领配给 → 开图开闸 → 上板（并开树那条会话）
//!   tree.rs      4–5：上树那一趟（`driver::tree::plate`）＋ 登记那一条线（`driver::register`）
//!   resident.rs  6  ：常驻——那条线一响就排空、交出去、说一句"我排空了"（**壳**）
//!   fail.rs      本域的死法（**下线**那一格）
//! ```
//!
//! **这一半由 bin 自己 `mod`**（不编进 lib）：本域的客人（`echo`）只经树拿到那枚孔，
//! **不读本域任何一份源码** ⇒ 连 `core/` 也在 bin 侧（与 `rtc` 那一侧相反，见 `driver/uart/mod.rs`）。
//! 死法住这里（不是 `uart/fail.rs`）：它实现 `programs::Exit`、报的是内核出口那一行——程序侧的事。

pub mod boot;
pub mod fail;
pub mod resident;
pub mod tree;
