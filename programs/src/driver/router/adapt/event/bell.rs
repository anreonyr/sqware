//! 领一条 → 往主人手里响一位 → 响到了才静音 ＋ 结 → 报一行。
//! 判定在 crate::core::lines（`deliver` 与 `told`）；静音/结是设备面的一手
//! （`plic.disable` / `plic.complete`）。
//! **领到空**——不按 `bell.wait(0)` 的返回值判。那一位是**中断闸门的账**（响着 ⇒ 本 hart 的
//! `SEIE` 关着），而组那一次等待会与它互相消费 ⇒ 按返回值进门就会漏掉"响过、但组先把它取走了"
//! 的那一次，**闸门从此关着，一枚中断都不再来**（实测：字节留在设备里、PLIC 的 `pending` 一直
//! 应铃那一手在 `resident` 里**无条件**做。

use crate::core::lines::Lines;
use crate::dev::plic::Plic;
use protocol::debug;

pub fn ring(lines: &mut Lines, plic: &Plic) {
    loop {
        let line = plic.claim();
        if line == 0 {
            break;
        }
        // **静音只在"那一帧真的送到了"之后**：投不出去（客户的口封了）就不静音
        let sent = lines.deliver(line).is_ok();
        if sent {
            plic.disable(line);
        } else {
            debug!("router: deliver failed line={line}");
        }
        // **（临时读数）**：领到哪条线 ＋ 投出去没有 ⇒ 静音了没有。**静音之后就等客人那句
        // "我排空了"**（`exhaust` 那一支才 `plic.enable`）——故"静音了、此后一条 enable 都没有"
        // 就是"这条线再也不会报中断"那件事的原文。前 20 次打全。
        {
            static N: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                protocol::debug::put(&alloc::format!("router: irq line={line} sent={sent}"));
            }
        }
        // 这条线的**第一次**：打一行只可能由中断链产生的读数（见 `driver/router/mod.rs`）。
        // **行首先补一个换行**：这一行是**兜底**——根因（一条读数行要 2~5 次 ecall、
        if lines.told(line) {
            debug!("\nrouter: line={line}");
        }
        plic.complete(line);
    }
}
