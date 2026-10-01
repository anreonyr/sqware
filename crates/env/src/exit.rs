//! 退场**原因码**：**内核与域共用的一张表**。
//!
//! `Reap { reason }` 送的就是这里的 [`Reason`]——内核只把它记进 trace、**不解释语义**
//! （见 `runtime::env::room::exit`）。约定：
//!
//! - `0`（[`EXIT_OK`]）= 自愿/正常结束；
//! - `1..` 小整数 = **域自己的**诊断编号（各 bin 用 `1`、`2`… 标明死在启动握手的哪一步；
//!   装配那一族在 `programs::unit` 的 `E_*`）；
//! - 高位段 = **ABI 的**（[`EXIT_PANIC`] / [`EXIT_FAULT`]），与上面两族不重叠，
//!   读 trace 的人一眼能分出"启动没走通"与"域自己炸了"。
//!
//! **域侧的"出口形状"不在本文件**：`main` 的返回类型 `Report`、那个 `Exit`
//! trait，加"把它送进内核"的 `finish`——三者住 `runtime::core::exit`（那边是 `runtime`，
//! 本 crate 够不着，故这里只写路径、不做链接）。分界只有一条，而且是可 grep 的：
//! **本文件只放内核也读的东西**。内核读 [`Reason`] 与 [`EXIT_FAULT`]
//! （`kernel/src/work/room/messenger/mod.rs` 直接 `pub(crate) use env::EXIT_FAULT`），
//! 而内核**不依赖 `runtime`**（方向 `kernel → env → runtime`）⇒ 码表必须住本 crate；
//! 反过来"码从哪来、话怎么带、往哪送"是域自己的事，内核一处也不碰。

/// 退场原因码——`usize`，直接就是 `Reap { reason }` 送出去的那个数。
///
/// 别名而非 newtype：它要跨 ABI（内核只当数据），而"是哪一族的编号"由**域自己的
/// 编号表**说话，不由类型说话（各 bin 的 `const E_*`、`programs::unit` 的 `E_*`）。
pub type Reason = usize;

/// 自愿／正常结束（"没有失败要报"）。
pub const EXIT_OK: Reason = 0;

/// **域内 panic**：`programs/src/entry.rs` 的 panic handler 专用。
///
/// 取高位段、与各 bin 的启动握手编号（小整数）不重叠——读 trace 的人一眼能分出
/// "启动没走通"与"域自己炸了"。
pub const EXIT_PANIC: Reason = 0xFFFF_FF01;

/// **内核给的故障原因**：用户引起的异常、未知调用号一类，由内核的故障隔离路径落账
/// （`kernel/src/runtime/switcher/trap/mod.rs` 那两处）。域自己永远不会传这个数。
pub const EXIT_FAULT: Reason = 0xFFFF_FFFF;
