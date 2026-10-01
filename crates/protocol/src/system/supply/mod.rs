//! supply — **物料到手**：引导域向上层露的那一面（**按坐标发货**）。
//! 它是 [`crate::system`] 的**供给侧**：这一层记的是"引导域交给域的一切"。**不全是设备**——
//! 今天五笔货分两类：
//! ```text
//!   设备侧（驱动领）    那几台设备的 `reg` 段（Pole，按**区**取）
//!                      irq（Nole，空载荷的中断门铃，按"哪一件"取）
//!   纯物料（谁用谁领）  devicetree（自描述区，router 领，按"哪一件"取）
//!                      initrd（载荷区，编排域领，按**区**取）
//! ```
//! 后两笔**借这条路**，不因此变成设备（"这张账记的是交出了哪些门闩，不是有哪些设备"）。

pub mod frame;

pub mod client;

// 形、据**转出**（`crate::system::supply::{frame,core}` 照旧解析）。
pub use crate::system::supply::frame::{BOOT, OP_SUPPLY, ORDER_CAP, REPLY_CAP, WANT_MAX};
pub use frame::Fail;
