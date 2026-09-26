//! router::adapt::resident — **常驻· 壳**：等三源 → 四手各就位。
//!
//! 判定不在这里：账与四原语住 `contract::driver::line::core`，"区 ↔ 线号"住 `crate::core::sources`，
//! 每一次醒来的四件事各有一份（`sweep` / `exhaust` / `desk` / `bell`）——本文件只做"等、取、喂"。

use super::boot::Up;
use super::{bell, desk, exhaust, sweep};
use super::fail;
use env::Wait;

/// 常驻：**一只组等两个源**（加上门牌，共三个）。
///
/// 失败：组坏了 ⇒ `Err(Fail::Bell)`——本域没有可继续的状态（铃那一格今天不可达：它的资源实体
/// 由内核**永久持有**，`platform/devices.rs::IRQ`——它是一格防御，不是读数）。
pub fn run(up: &mut Up) -> Result<(), fail::Fail> {
    loop {
        // **等到有事件**：三样（铃 / 门上有人 / 客人的排空）都可等地，醒来就说明有一格有事。
        //
        // **纯事件（`Wait::Forever`），没有兜底的一拍**：旧写法带 20 ms 期限，为的是盖住"偶尔
        // 一次组等待没被叫醒"（实测：PLIC 的 `pending` 置着、本域不再被叫醒，字节留在设备里）。
        // 那一格的根在铃那一侧——空闲核不进外部 trap，`raise_irq` 在它身上没有调用点，铃
        // 根本没响（见 `kernel/src/work/room/scheduler/core/fetch.rs` 的空闲循环）。根修在
        // 那里，`SEIP` 能挂的那两条长驻态各有振铃点之后这一拍就是多余的：铃一定响，
        // 醒来 `claim`+`hush` 即到。
        match up.pile.await_(Wait::Forever) {
            Ok(Some(_)) => {}
            // 挂起过（不是期限）：照样往下走一遍——`claim` 领到空就什么也不做。
            Ok(None) => {}
            Err(_) => return Err(fail::Fail::Bell),
        }
        // 逐客：**每次醒来扫一遍有主的那些条**——主人没了就拆线 + 空出格子。放在最前：
        // 那一格收掉之后再取排空、再登记，账里就只剩还活着的客人。
        sweep::run(&mut up.lines, &up.plic, &up.pile);
        // 排空：客人说一句"这一条我排空了" ⇒ 那一格回闲 + **把线放回去**（事件，不是节拍）。
        // **先取排空，再登记**：登记会把新的一条线接上，紧接着到来的那一枚中断才不漏。
        exhaust::drain(&mut up.lines, &up.plic, &mut up.buf);
        // 门上：非阻塞地把槽里的都取走（登记）。缓冲是**一页**（载体的界，见 `Push` 的前置
        // 条件）——于是任何一条消息一趟都取得出来，"取不出也丢不掉"那个状态不存在。
        while let Ok((n, from)) = up.entry.pull_timeout_from(&mut up.buf, Wait::POLL) {
            desk::serve(
                &mut up.lines,
                &up.plic,
                &up.sources,
                from,
                &up.buf[..n],
                &up.pile,
            );
        }
        // 铃：领干净这一趟（`bell` 那一份里写着"为什么不按铃的返回值判"）。
        bell::ring(&mut up.lines, &up.plic);
        // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。**无条件**做——
        // 没响时它答 `Busy`（幂等），而少做一次就是闸门永久关着。
        let _ = up.bell.hush();
    }
}
