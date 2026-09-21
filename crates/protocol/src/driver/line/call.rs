//! line::call — **帧形与记号**：两句话，两份形状。
//!
//! ```text
//!   登记（门牌那条路上一问一答）  [OCCUPY][设备名 32B]   →  [状态码 1B]
//!   线泊位（路由者 ↔ 客户）       [记号 1B]             两个方向同一份形状
//! ```
//!
//! 投递与排空是**同一条路的两个方向**，故不带动作码、也不带线号——方向就是那一格，而**泊位
//! 就是坐标**（客户手里没有"线"，见 [`super::mod`]）。那 1 字节不是信息，是**形状的下限**：
//! 孔不收 0 字节的报文。登记那一句与"招呼"共用一扇门，故只有它带动作码。

use env::Name;
use env::wire::NAME_LEN;

use super::core::Fail;

/// 登记那一句的动作码。
pub const OCCUPY: u8 = 1;

/// 登记帧的长度（动作码 + 设备名）。
pub const ASK: usize = 1 + NAME_LEN;

/// 线泊位的记号（两侧同一个）。
pub const LANE: &str = "line";

/// 回信孔的记号（登记那一答从它回来）。
pub const BACK: &str = "line-back";

/// 线泊位两个方向那一个记号：**帧不报内容，只报"有事"**（形状的下限，见文件头）。
pub const NOTE: u8 = 1;

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
pub fn pack_occupy(device: Name) -> [u8; ASK] {
    let mut out = [0u8; ASK];
    out[0] = OCCUPY;
    out[1..].copy_from_slice(device.bytes());
    out
}

/// 拆一帧登记：**不是那个形状就答 `None`**（这一扇门还兼着"招呼"那一形状，故不猜）。
pub fn unpack_occupy(frame: &[u8]) -> Option<Name> {
    if frame.len() != ASK || frame[0] != OCCUPY {
        return None;
    }
    let raw: [u8; NAME_LEN] = frame.get(1..ASK)?.try_into().ok()?;
    Name::from_bytes(raw).ok()
}
