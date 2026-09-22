//! system::grant — **配给那一半**：装配者把门闩交到子域手里的那段记录。
//!
//! 与固件面的回单**同一种字节**（`Pair` 数组：坐标 + 号）——两条路记的是同一件事：
//! "这东西**种在你表里的那个号**"。故这里只做**线格式那一半**：把一段字节按步长解出来，
//! 一条一条交给调用方；**第 i 条就是单子第 i 条的答**（同序同长）——收方那张表按位次归位，
//! 本模块不解释任何坐标。
//!
//! 发的那一半在装配者手里：它向固件**领**（`protocol::driver::supply::client::draw`）拿到这段字节，
//! 再**原样**推进子域那条通道——本模块不碰原件，也不认识设备。

use env::{PAIR_LEN, Pair};

/// 把一段记录解出来，**第 i 条交给第 i 格**（位置即格）。
///
/// 前置：`records.len()` 是 [`PAIR_LEN`] 的整数倍（调用方先按"回单与单子同序同长"校验过条数）。
/// 缓冲只保证 1 字节对齐，故逐条 `read_unaligned`（与 `Pair` 的其余两处同一条理由）。
pub fn each(records: &[u8], mut f: impl FnMut(usize, Pair)) {
    for i in 0..records.len() / PAIR_LEN {
        // SAFETY: 记录与块同源（`Pair` 的尺寸由编译期断言锁死为 `PAIR_LEN`）；缓冲只保证
        // 1 字节对齐，故 `read_unaligned`。越界由上面的除法挡掉。
        let at = unsafe { records.as_ptr().add(i * PAIR_LEN) };
        let record = unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) };
        f(i, record);
    }
}
