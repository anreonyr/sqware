//! router::adapt::bell — **铃（适配）**：领一条 → 往主人手里响一位 → 响到了才静音 ＋ 结 → 报一行。
//! 判定在 `crate::core::lines`（`deliver` 与 `told`）；静音/结是设备面的一手
//! （`plic.disable` / `plic.complete`）。
//! **领到空**——不按 `bell.wait(0)` 的返回值判。那一位是**中断闸门的账**（响着 ⇒ 本 hart 的
//! `SEIE` 关着），而组那一次等待会与它互相消费 ⇒ 按返回值进门就会漏掉"响过、但组先把它取走了"
//! 的那一次，**闸门从此关着，一枚中断都不再来**（实测：字节留在设备里、PLIC 的 `pending` 一直
//! 置着，而本域睡到天荒地老）。故这里直接领：领到空就什么也不做，领到就投递 + 结，
//! 应铃那一手在 `resident` 里**无条件**做。

use crate::core::lines::Lines;
use crate::dev::plic::Plic;
use protocol::debug;

/// 领干净这一趟铃：每条领到的线响一位、静音、报过没有、结清。
pub fn ring(lines: &mut Lines, plic: &Plic) {
    loop {
        let line = plic.claim();
        if line == 0 {
            break;
        }
        // **静音只在"那一帧真的送到了"之后**：投不出去（客户的口封了）就不静音
        // ——静音是"这一条有人接了"，而没人接的那一条不该由本域替它按下。
        if lines.deliver(line).is_ok() {
            plic.disable(line);
        } else {
            debug!("router: deliver failed line={line}");
        }
        // 这条线的**第一次**：打一行只可能由中断链产生的读数（见 `driver/router/mod.rs`）。
        // **行首先补一个换行**：这一行是**兜底**——根因（一条读数行要 2~5 次 ecall、
        if lines.told(line) {
            debug!("\nrouter: line={line}");
        }
        plic.complete(line);
    }
}
