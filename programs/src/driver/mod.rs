//! 每台的树里：dev/ 设备面（唯一碰 MMIO）· core/ 纯功能 · adapt/ 住持面 · main.rs 只剩流程。
//! 判据是**角色**，不是特权级（与测具那一档 `src/harness/` 同款）：本目录下的成员各自在

pub mod router;
pub mod rtc;
pub mod shared;
pub mod uart;
