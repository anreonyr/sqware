//! uart — NS16550 的**最小设备面**：只够一件事——开关"收到字节就拉中断线"。
//!
//! 它**不是** UART 驱动：不读字节、不写字节、不管 FIFO、不管行。读写今天归内核的调试面
//! （`DebugCall` → SBI），而 `IER` 只控制**中断线**、不控制数据通路——故打开这一位不会把
//! 字符从谁手里抢走。
//!
//! 存在的理由只有一个：第一刀需要一个**能响的中断源**，否则"中断到 → 内核摇铃 → 域醒"
//! 这一段没有读数（SBI 轮询读串口，那一页自己从不拉线）。第二刀控制台搬过来时，这一位
//! 归 UART 驱动域。

use runtime::core::dock::View;

/// `IER` = 中断使能寄存器（NS16550 的寄存器偏移 1）。
///
/// **只动这一格**，故不需要 DLL/DLM 那套"偏移 1 的双重身份"（DLAB=1 时它才是波特率低字节，
/// 而本域从不设 DLAB）。
const IER: usize = 1;

/// `IER.RX`：接收数据可用 ⇒ 拉中断线。
const IER_RX: u8 = 0x01;

/// 打开"收到字节就拉线"。
pub fn arm_rx(view: View) {
    // SAFETY: `view` 是 `Dock::open` 的产物——UART 那段已借映进本域；只写该寄存器。
    unsafe { core::ptr::write_volatile((view.base() + IER) as *mut u8, IER_RX) }
}
