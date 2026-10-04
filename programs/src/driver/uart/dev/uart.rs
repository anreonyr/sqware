//! 最小设备面：够四件事——开"收到字节就拉线"、问"有没有字节"、把字节取走、
//! 把一条字塞出去。

use runtime::core::res::dock::View;

/// `RBR` = 接收缓冲：**读它就是取走一个字节**（`LSR.DR` 随之落）
const RBR: usize = 0;

/// `THR` = 发送保持寄存器：**写它就是塞一个字节出去**（与 `RBR` 同一个偏移的写侧）
const THR: usize = 0;

/// `IER` = 中断使能寄存器（NS16550 的寄存器偏移 1）
const IER: usize = 1;

/// `LSR` = 线路状态；第 0 位 `DR`
const LSR: usize = 5;

/// `IER.RX`：接收数据可用 ⇒ 拉中断线
const IER_RX: u8 = 0x01;

/// `LSR.DR`：收一个字节到了
const LSR_DR: u8 = 0x01;

/// `LSR.THRE`：发送保持寄存器空了（可以塞下一个字节）
const LSR_THRE: u8 = 0x20;

/// 打开"收到字节就拉线"——**读改写**：只置 `RX` 这一位
pub fn arm_rx(view: View) {
    let at = (view.base() + IER) as *mut u8;
    unsafe {
        let bits = core::ptr::read_volatile(at);
        core::ptr::write_volatile(at, bits | IER_RX);
    }
}

/// 排空：`LSR.DR` 还置着就把 `RBR` 读走，返读到的字节数（**0 也是读数**：这一批没有内容）
/// 之后，路由者把线放回去，电平还高 ⇒ 立刻再报）
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

/// 塞出去：`LSR.THRE` 置着就写一个字节进 `THR`，返塞出去的字节数（= `bytes.len()`）
/// **一次写 = 一条完整的字**由服务台那一侧保证（一条消息就是全部字节）；本手只负责**照原样**
/// 把这几个字节塞进设备，不拆、不并、不添字
/// **这一圈等的是有界的东西**（不是"空转的红线"那一档）：等的是我们自己要塞的这几个字节
/// 而 `put` 的用家（`desk`）一条字 ≤ 那一面的传输上限（`core::frame::MAX`）
/// ⇒ 115200 波特下最坏约 11 ms。设备里的字节
/// 印出来这件事只能由持有者做——固件那一侧 Dbcn::ConsoleWrite 也是这么等的
pub fn put(view: View, bytes: &[u8]) -> usize {
    let at = view.base() as *const u8;
    let mut n = 0;
    // SAFETY: 同 `arm_rx`/`drain`；只读写 `LSR` / `THR` 两格（写 `THR` 即塞出一个字节）。
    unsafe {
        while n < bytes.len() {
            while core::ptr::read_volatile(at.add(LSR)) & LSR_THRE == 0 {}
            core::ptr::write_volatile(at.add(THR) as *mut u8, bytes[n]);
            n += 1;
        }
    }
    n
}
