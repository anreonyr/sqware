//! 客人说一句"这一条我排空了" ⇒ 那一格回闲 + 把线放回去。
//! 判定在 crate::core::lines（`exhaust` 那一手）；放线是设备面的一手
//! （`plic.enable`）。
//! 对账，响着就是那一条。

use crate::core::lines::Lines;
use crate::dev::plic::{LINE_PRIORITY, Plic};
use runtime::env::mail::HolePie;

/// 排空：取"忙"的那些，把响着的那一位应掉，每条回闲 + 放线。
/// **位只有一位**：一条线上"我排空了"是**状态**不是事件——两次排空合成一次（`lines.exhaust`
pub fn drain(lines: &mut Lines, plic: &Plic) {
    // **取"忙"的那些**（不是"有主"的那些）：只有"响过、还没回闲"的那一格才欠一句
    // 排空；这一句也是那个 `忙` 的唯一读者——账上那一格因此不是写给别人看的。
    let busy: alloc::vec::Vec<u32> = lines.busy().collect();
    for line in busy {
        let Some(lane) = lines.lane(line) else {
            continue;
        };
        // 应**本端那一枚**（客人往它响"我排空了"）；号先取出来，下面那一手要改账（借不动）。
        let rx = lane.rx();
        while HolePie::from_token(rx).hush().is_ok() {
            let _ = lines.exhaust(line);
            plic.enable(line, LINE_PRIORITY);
        }
    }
}
