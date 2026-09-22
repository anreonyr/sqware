//! router::needs — **本域自己那片硬件账**：要哪几枚门闩、什么种类/权/形态。
//!
//! 它住在本域里，因为它是**收方**开的那张单子：装配者照它开单（[`WANTS`] 那几条原样递出去），
//! 本域收到记录后**按位次归位**（[`crate::driver::assemble::receive`]——位置即格）。
//!
//! **两类坐标都在这一张单上**：控制器按**类**要（`sifive,plic-1.0.0`，树里认）；设备树本体与
//! 门铃按**名**要——它们不是设备树里的节点，是内核造的门闩（`devicetree` / `irq`）。
//!
//! **只要三枚**：控制器、自描述、门铃。那台串口归 [`crate::driver::uart`]——**线的闸门归设备
//! 持有者**，故本域不去替它领（`ONLY` 是资源事实，一张表里只能有一个持有者）。

use protocol::driver::supply::call::{Kind, Need, class_block, name_block};
use runtime::core::port::{Access, Policy};

/// 本域认控制器的那个类（`compatible` 串）——**只此一处**：单子上按它要，
/// [`super::plic`] 按它找节点读属性，两侧同一个字。
pub const PLIC: &str = "sifive,plic-1.0.0";

/// 本域那张表里的第几格（判别号 = 数组下标；归位按位次 ⇒ 两者同值）。
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

/// 本域要的三枚 —— **直接就是单子上的那几条**。
pub const WANTS: &[Need] = &[
    Need::class(
        class_block(PLIC),
        Kind::Pole,
        Access::FETCH_STORE,
        Policy::ONLY,
    ),
    Need::named(
        name_block("devicetree"),
        Kind::Pole,
        Access::FETCH,
        Policy::NONE,
    ),
    Need::named(name_block("irq"), Kind::Nole, Access::FETCH, Policy::NONE),
];
