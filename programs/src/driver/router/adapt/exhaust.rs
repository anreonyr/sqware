//! router::adapt::exhaust — **排空（适配）**：客人说一句"这一条我排空了" ⇒ 那一格回闲 + 把线放回去。
//!
//! 判定在 `contract::driver::line::core`（`exhaust` 那一手）；放线是设备面的一手
//! （`plic.enable`）。
//!
//! **按泊位认线**：一条线一枚泊位，谁推的那一枚就是哪一条——**帧里没有线号**（1 字节记号，
//! 见 [`protocol::driver::line::frame`]），故这里无需对账，取到就是那一条。
//!
//! 非阻塞取干净再回去等：`pull(.., 0)` 期限内没有就是没有，**不是错误**。

use crate::plic::{LINE_PRIORITY, Plic};
use env::Wait;
use protocol::driver::line::core::Lines;

/// 排空：取"忙"的那些，把里面的通知取干净，每条回闲 + 放线。
///
/// `buf` = **门外那一页**（调用方那只，见 `resident`）：道上的记号虽然只有 1 字节，
/// 缓冲仍按**载体**的界备——一枚更长的推落进道里时，1 字节的读法取不出也丢不掉，这一条线
/// 就永远回不了闲（`exhaust` 那一句再也不会跑，线也不会重开）。
pub fn drain(lines: &mut Lines, plic: &Plic, buf: &mut [u8]) {
    // **取"忙"的那些**（不是"有主"的那些）：只有"投出去过、还没回闲"的那一格才欠一句
    // 排空；这一句也是那个 `忙` 的唯一读者——账上那一格因此不是写给别人看的。
    let busy: alloc::vec::Vec<u32> = lines.busy().collect();
    for line in busy {
        let Some(lane) = lines.lane(line) else {
            continue;
        };
        while lane.pull(buf, Wait::POLL).is_ok() {
            let _ = lines.exhaust(line);
            plic.enable(line, LINE_PRIORITY);
            // debug!("router: exhaust line={line}");
        }
    }
}
