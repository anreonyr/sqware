//! driver — **驱动这一族**：设备面的持有者，与它们共用的装配件。
//! 判据是**角色**，不是特权级（与 `harness`（测具那一个 crate） 同款）：本目录下的成员各自在

pub mod shared;
pub mod router;
pub mod rtc;
pub mod uart;
