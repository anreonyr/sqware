//! system::grant — **配给那一半**：装配者把门闩交到子域手里的那段记录。
//!
//! 与固件面的回单**同一种字节**（`Pair` 数组：名字块 + 号）——两条路记的是同一件事：
//! "这东西**种在你表里的那个号**"。故这里只做**线格式那一半**：把一段字节按步长解出来，
//! 一条一条交给调用方；"名字 → 我表里第几格"是**那台机器**的账（它的需求单），由调用方给。
//!
//! 发的那一半在装配者手里：它向固件**领**（`protocol::firmware::client::draw`）拿到这段字节，
//! 再**原样**推进子域那条通道——本模块不碰原件，也不认识设备。

use env::{PAIR_LEN, Pair, PieToken};

/// 把一段记录解出来，一条一条交给 `f`：`slot_of` 把名字翻成"收方那本账里的第几格"。
///
/// 名字翻不出来的条目**直接跳过**（不是错：单子只增不减时会有旧记录）。
/// 缓冲只保证 1 字节对齐，故逐条 `read_unaligned`（与 `Pair` 的其余两处同一条理由）。
pub fn unpack(
    records: &[u8],
    slot_of: impl Fn(&str) -> Option<usize>,
    mut f: impl FnMut(usize, PieToken),
) {
    for i in 0..records.len() / PAIR_LEN {
        // SAFETY: 记录与块同源（`Pair` 的尺寸由编译期断言锁死为 `PAIR_LEN`）；缓冲只保证
        // 1 字节对齐，故 `read_unaligned`。越界由上面的除法挡掉。
        let at = unsafe { records.as_ptr().add(i * PAIR_LEN) };
        let rec = unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) };
        let Some(name) = rec.name() else { continue };
        if let Some(slot) = slot_of(name.as_str()) {
            f(slot, rec.token());
        }
    }
}
