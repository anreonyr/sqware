//! line::call — **帧形与记号**：两句话，两份形状。
//!
//! ```text
//!   登记（门牌那条路上一问一答）  [OCCUPY][设备名 32B]   →  [状态码 1B]
//!   线泊位（路由者 ↔ 客户）       [记号 1B]             两个方向同一份形状
//! ```
//!
//! 投递与排空是**同一条路的两个方向**，故不带动作码、也不带线号——方向就是那一格，而**泊位
//! 就是坐标**（客户手里没有"线"，见 [`super::mod`]）。那 1 字节不是信息，是**形状的下限**：
//! 孔不收 0 字节的报文。登记那一句带一个动作码（`OCCUPY`）——**照实记**：它原来是为了与"招呼"
//! 那一形状分开（同一扇门、靠帧长分），那条形状已经退休。**留着**（用户裁定）：这一格不是死
//! 字段——[`unpack_occupy`] 真的读它（形状不对就答 `None`，路由器不动账），而且本仓**三扇门
//! 都带动作码**（板那一扇的 `REGISTER` 一族、树那一扇的 `land`/`find` 一族）——去掉只省
//! 1 字节，换来"任何恰好 32 字节推上来的东西都算一次登记"。

use env::Name;
use env::wire::NAME_LEN;

use super::core::Fail;

/// 登记那一句的动作码。
pub const OCCUPY: u8 = 1;

/// 登记帧的长度（动作码 + 设备名）。
pub const OCCUPY_LEN: usize = 1 + NAME_LEN;

/// 线泊位的记号（两侧同一个）。
pub const LANE: &str = "line";

/// 回信孔的记号（登记那一答从它回来）。
pub const BACK_MARK: &str = "line-back";

/// 线泊位两个方向那一个记号：**帧不报内容，只报"有事"**（形状的下限，见文件头）。
pub const NOTE: u8 = 1;

/// 状态码与失败域**同源**：一格对应一个不同的下一步。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const BAD: u8 = 4;

/// 失败域 → 状态码（**一处编**：客户与路由者看同一张表）。`None`（没失败）⇒ `OK`。
pub const fn fail_to_code(fail: Option<Fail>) -> u8 {
    match fail {
        None => OK,
        Some(Fail::Unknown) => UNKNOWN,
        Some(Fail::Taken) => TAKEN,
        Some(Fail::Denied) => DENIED,
    }
}

/// 状态码 → 失败域。`OK`（没失败）与 `BAD`（这一帧读不懂）**都不是失败域里的东西**，
/// 故两者同一格答 `None`——读的人靠 [`unpack_occupy`] 先分流。
///
/// **本表是双射**（三个失败一格一码），故反向答得回来。
pub const fn code_to_fail(code: u8) -> Option<Fail> {
    match code {
        UNKNOWN => Some(Fail::Unknown),
        TAKEN => Some(Fail::Taken),
        DENIED => Some(Fail::Denied),
        _ => None,
    }
}

/// 登记帧：动作码 + 设备名。
pub fn pack_occupy(device: Name) -> [u8; OCCUPY_LEN] {
    let mut out = [0u8; OCCUPY_LEN];
    out[0] = OCCUPY;
    out[1..].copy_from_slice(device.bytes());
    out
}

/// 拆一帧登记：**不是那个形状就答 `None`**（别人往这扇门推别的东西时，不猜）。
pub fn unpack_occupy(frame: &[u8]) -> Option<Name> {
    if frame.len() != OCCUPY_LEN || frame[0] != OCCUPY {
        return None;
    }
    let raw: [u8; NAME_LEN] = frame.get(1..OCCUPY_LEN)?.try_into().ok()?;
    Name::from_bytes(raw).ok()
}
