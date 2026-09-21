//! line::call — **帧形与记号**：三句话，一份形状。
//!
//! ```text
//!   登记（门牌那条路上一问一答）  [RESERVE][设备名 32B]   →  [状态码 1B]
//!   投递（泊位：路由者 → 客户）    [线号 4B]
//!   排空（泊位：客户 → 路由者）    [线号 4B]
//! ```
//!
//! 投递与排空是**同一条路的两个方向**，故不带动作码（方向就是那一格）；登记那一句与"招呼"
//! 共用一扇门，故带一个动作码。

use env::Name;
use env::wire::NAME_LEN;

use super::core::Fail;

/// 登记那一句的动作码。
pub const RESERVE: u8 = 1;

/// 登记帧的长度（动作码 + 设备名）。
pub const ASK: usize = 1 + NAME_LEN;

/// 线泊位的记号（两侧同一个）。
pub const LANE: &str = "line";

/// 回信孔的记号（登记那一答从它回来）。
pub const BACK: &str = "line-back";

/// 状态码与失败域**同源**：一格对应一个不同的下一步。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const BAD: u8 = 4;

/// 失败域 → 状态码（**一处编**：客户与路由者看同一张表）。
pub fn code_of(fail: Fail) -> u8 {
    match fail {
        Fail::Unknown => UNKNOWN,
        Fail::Taken => TAKEN,
        Fail::Denied => DENIED,
    }
}

/// 登记帧：动作码 + 设备名。
pub fn pack_reserve(device: Name) -> [u8; ASK] {
    let mut out = [0u8; ASK];
    out[0] = RESERVE;
    out[1..].copy_from_slice(device.bytes());
    out
}

/// 拆一帧登记：**不是那个形状就答 `None`**（这一扇门还兼着"招呼"那一形状，故不猜）。
pub fn unpack_reserve(frame: &[u8]) -> Option<Name> {
    if frame.len() != ASK || frame[0] != RESERVE {
        return None;
    }
    let raw: [u8; NAME_LEN] = frame.get(1..ASK)?.try_into().ok()?;
    Name::from_bytes(raw).ok()
}

/// 投递 / 排空那一帧：一个线号（小端）。
pub fn pack_line(line: u32) -> [u8; 4] {
    line.to_le_bytes()
}

/// 拆一帧投递 / 排空；形状不对 ⇒ `None`。
pub fn unpack_line(frame: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(frame.get(..4)?.try_into().ok()?))
}
