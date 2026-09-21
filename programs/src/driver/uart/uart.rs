//! uart — NS16550 的**最小设备面**：够三件事——开"收到字节就拉线"、问"有没有字节"、把字节取走。
//!
//! 它是**本域（串口驱动）的设备面**：`IER` 只控制**中断线**、不控制数据通路，故"开这一位"
//! 不会把字符从谁手里抢走；而**读**（[`drain`]）才是"把字节从设备里取走"那一手——它从前在
//! 内核的调试面（固件代读 `RBR`），今天在持有设备的本域。**一台设备只有一个读者**。
//!
//! 不碰 FIFO 配置、不管行、不解释字节：FIFO 是固件初始化时开好的（`uart8250_device_init`），
//! "一行"是**终端**的约定（在客人那侧，见 `programs/src/user/echo.rs`）。

use runtime::core::dock::View;

/// `RBR` = 接收缓冲：**读它就是取走一个字节**（`LSR.DR` 随之落）。
const RBR: usize = 0;

/// `IER` = 中断使能寄存器（NS16550 的寄存器偏移 1）。
///
/// **只动这一格**，故不需要 DLL/DLM 那套"偏移 1 的双重身份"（DLAB=1 时它才是波特率低字节，
/// 而本域从不设 DLAB）。
const IER: usize = 1;

/// `LSR` = 线路状态；第 0 位 `DR`。
const LSR: usize = 5;

/// `IER.RX`：接收数据可用 ⇒ 拉中断线。
const IER_RX: u8 = 0x01;

/// `LSR.DR`：收一个字节到了。
const LSR_DR: u8 = 0x01;

/// 打开"收到字节就拉线"——**读改写**：只置 `RX` 这一位。
///
/// 整格写（`= IER_RX`）会把这一格里别的位一起抹掉；今天别的位本来就是 0（内核与固件都不
/// 动 IER），但"只动这一格"这句话要真的成立就得读改写。DLAB 恒为 0（本域从不设它），
/// 故读到的就是 `IER` 本体，不是波特率低字节。
pub fn arm_rx(view: View) {
    let at = (view.base() + IER) as *mut u8;
    // SAFETY: `view` 是 `Dock::open` 的产物——UART 那段已借映进本域；只读写该寄存器。
    unsafe {
        let bits = core::ptr::read_volatile(at);
        core::ptr::write_volatile(at, bits | IER_RX);
    }
}

/// 排空：`LSR.DR` 还置着就把 `RBR` 读走，返读到的字节数（**0 也是读数**：这一批没有内容）。
///
/// 一次读到一个字节**就是**"从设备里取走它"——连着的那些字节 FIFO 里排着，故这里循环到
/// `DR` 落为止。装满 `out` 就停：剩下的还在设备里，**下一次中断再来**（本域说一句"排空了"
/// 之后，路由者把线放回去，电平还高 ⇒ 立刻再报）。
pub fn drain(view: View, out: &mut [u8]) -> usize {
    let at = view.base() as *const u8;
    let mut n = 0;
    // SAFETY: 同 `arm_rx`；`LSR.DR` 置着才有东西可读，读 `RBR` 即取走一个字节。
    unsafe {
        while n < out.len() && core::ptr::read_volatile(at.add(LSR)) & LSR_DR != 0 {
            out[n] = core::ptr::read_volatile(at.add(RBR));
            n += 1;
        }
    }
    n
}
