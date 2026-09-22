//! uart::needs — **本域自己那片硬件账**：要哪一类设备、什么种类/权/形态。
//!
//! 它住在本域里，因为它是**收方**开的那张单子：装配者照它开单（[`WANTS`] 那几条原样递出去），
//! 本域收到记录后**按位次归位**（[`crate::driver::assemble::receive`]——位置即格）。
//!
//! **写的是类，不是名字**：`ns16550a` 是这台设备的绑定名（树里写在 `compatible` 上），
//! 而"这一类是哪一台"由编排域读树定下来（`supervisor::system::machine`）——**本域不发明名字，
//! 也不冻机器地址**。**坐标**（那一段区）随记录回到本域手里（报线要它），但它不由本域写死。

use protocol::driver::supply::call::{Kind, Need, class_block};
use runtime::core::port::{Access, Policy};

/// 本域要的那一枚 —— **直接就是单子上的那一条**。
///
/// 类取 `ns16550a`（这台串口的绑定名）：**设备的类是驱动的专业**，机器把它摆在哪是树的事。
///
/// `ONLY`（独占）是**资源事实**：一台设备的寄存器页同一时刻只该有一个持有者——谁来持有它，
/// 谁才有资格动它（包括"收到字节就拉线"那一位 `IER`）。故这一枚由**本域**从装配者手里领，
/// 而不是由线路由者代领。
pub const WANTS: &[Need] = &[Need::class(
    class_block("ns16550a"),
    Kind::Pole,
    Access::FETCH_STORE,
    Policy::ONLY,
)];
