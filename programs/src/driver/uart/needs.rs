//! uart::needs — **本域自己那片硬件账**：要哪几枚门闩、什么种类/权/形态，以及"名字 → 本域第几格"。
//!
//! 它住在本域里，因为它是**收方**开的那张单子：装配者照它开单（[`WANTS`] 那几条原样递出去），
//! 本域照它归位（[`slot_of`] 交给 `grant::unpack`）。名字是 boot 在配对块里给的原样
//! （设备树节点的 basename，见 `kernel/src/platform/devices.rs`）——**本域不发明名字**。

use protocol::firmware::call::{Kind, Want, name_block};
use runtime::core::port::{Access, Policy};

/// 收方给这枚门闩起的名字（判别号 = 本域那张表的数组下标）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// 串口那一页寄存器。
    Serial = 0,
}

/// 本域要的那一枚 —— **直接就是单子上的那一条**。
///
/// `ONLY`（独占）是**资源事实**：一台设备的寄存器页同一时刻只该有一个持有者——谁来持有它，
/// 谁才有资格动它（包括"收到字节就拉线"那一位 `IER`）。故这一枚由**本域**从装配者手里领，
/// 而不是由线路由者代领。
pub const WANTS: &[Want] = &[Want::of(
    name_block("serial@10000000"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];

/// 与 [`WANTS`] **同序**的格子：第 i 条要的东西落在第 i 格。
const SLOTS: [Slot; 1] = [Slot::Serial];

/// 名字 → 本域那本账里的第几格（`grant::unpack` 的 `slot_of`）。
pub fn slot_of(name: &str) -> Option<usize> {
    WANTS
        .iter()
        .zip(SLOTS)
        .find(|(want, _)| want.name().is_some_and(|n| n.as_str() == name))
        .map(|(_, slot)| slot as usize)
}
