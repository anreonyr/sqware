//! router::needs — **本域自己那片硬件账**：要哪几枚门闩、什么种类/权/形态，以及"名字 → 本域第几格"。
//!
//! 它住在本域里，因为它是**收方**开的那张单子：装配者照它开单（[`WANTS`] 那几条原样递出去），
//! 本域照它归位（[`slot_of`] 交给 [`crate::driver::assemble::receive`]）。名字是 boot 在配对块
//! 里给的原样（设备树节点的 basename，见 `kernel/src/platform/devices.rs`）——**本域不发明名字**。
//!
//! **只要三枚**：控制器、自描述、门铃。那台串口（`serial@10000000`）归
//! [`crate::driver::uart`]——**线的闸门归设备持有者**，故本域不去替它领（`ONLY` 是资源事实，
//! 一张表里只能有一个持有者）。

use protocol::firmware::call::{Kind, Want, name_block};
use runtime::core::port::{Access, Policy};

/// 收方给这枚门闩起的名字（判别号 = 本域那张表的数组下标）。
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// 中断控制器那一页寄存器。
    Plic = 0,
    /// 设备树本体（只读自描述）。
    Dtb = 1,
    /// 中断门铃（空载荷）。
    Bell = 2,
}

/// 本域要的三枚 —— **直接就是单子上的那几条**（装配者原样递出，中间没有 `Need` 那一层）。
pub const WANTS: &[Want] = &[
    Want::of(
        name_block("interrupt-controller@c000000"),
        Kind::Pole,
        Access::FETCH_STORE,
        Policy::ONLY,
    ),
    Want::of(
        name_block("devicetree"),
        Kind::Pole,
        Access::FETCH,
        Policy::NONE,
    ),
    Want::of(name_block("irq"), Kind::Nole, Access::FETCH, Policy::NONE),
];

/// 与 [`WANTS`] **同序**的格子：第 i 条要的东西落在第 i 格。
const SLOTS: [Slot; 3] = [Slot::Plic, Slot::Dtb, Slot::Bell];

/// 名字 → 本域那本账里的第几格（`grant::unpack` 的 `slot_of`）。
pub fn slot_of(name: &str) -> Option<usize> {
    WANTS
        .iter()
        .zip(SLOTS)
        .find(|(want, _)| want.name().is_some_and(|n| n.as_str() == name))
        .map(|(_, slot)| slot as usize)
}
