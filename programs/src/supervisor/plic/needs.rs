//! plic::needs — **本域自己那片硬件账**：要哪几枚门闩、什么种类/权/形态，以及"名字 → 本域第几格"。
//!
//! 它住在本域里，因为它是**收方**开的那张单子：装配者照它开单（[`WANTS`] 那几条原样递出去），
//! 本域照它归位（[`slot_of`] 交给 `grant::unpack`）。名字是 boot 在配对块里给的原样
//! （设备树节点的 basename，见 `kernel/src/platform/devices.rs`）——**本域不发明名字**。
//!
//! 从前它住在 `supervisor/needs.rs`（"两端共用"的一处），中间还隔着一层 `Need → Want` 的转换；
//! 那一层是多余的：装配者从头到尾不碰 `slot`（它只把单子递出去），故现在**单子上的形状就是
//! 这张表**，转换那一步整个没了。

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
    /// 第一刀的测试源（UART 的中断使能位）。
    Source = 3,
}

/// 本域要的四枚 —— **直接就是单子上的那几条**（装配者原样递出，中间不再有 `Need` 那一层）。
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
    Want::of(
        name_block("serial@10000000"),
        Kind::Pole,
        Access::FETCH_STORE,
        Policy::ONLY,
    ),
];

/// 与 [`WANTS`] **同序**的格子：第 i 条要的东西落在第 i 格。
const SLOTS: [Slot; 4] = [Slot::Plic, Slot::Dtb, Slot::Bell, Slot::Source];

/// 名字 → 本域那本账里的第几格（`grant::unpack` 的 `slot_of`）。
pub fn slot_of(name: &str) -> Option<usize> {
    WANTS
        .iter()
        .zip(SLOTS)
        .find(|(want, _)| want.name().is_some_and(|n| n.as_str() == name))
        .map(|(_, slot)| slot as usize)
}
