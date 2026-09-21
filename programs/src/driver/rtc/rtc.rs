//! rtc — `goldfish-rtc` 的**最小设备面**：够三件事——读一次时间、武装下一次闹钟、把到点那一格清掉。
//!
//! 布局与语义照这台设备的约定（QEMU `hw/rtc/goldfish_rtc.c`），本域只碰其中七格：
//!
//! ```text
//!   0x00 TIME_LOW        读它 = 当前计数低 32 位，**同时把高 32 位锁存起来**
//!   0x04 TIME_HIGH       读它 = 上面锁存的那一半 ⇒ **读时间必须先低后高**
//!   0x08 ALARM_LOW       写它 = 闹钟低半格，**并且当场比较一次**（到点就立刻报）
//!   0x0c ALARM_HIGH      写它 = 闹钟高半格（**它自己不比**）
//!   0x10 IRQ_ENABLED     = 1 才许它拉线
//!   0x14 CLEAR_ALARM     撤闹钟
//!   0x18 ALARM_STATUS    读它 = "**闹钟武装着**"（`alarm_running`），**不是**"到点了"
//!   0x1c CLEAR_INTERRUPT 清 `irq_pending`（**电平源**：不清，线就一直挂着）
//! ```
//!
//! **两条量出来的规矩**（都是先按想当然写、被读数打回来才改的，见本域头注）：
//!
//! - **读时间先低后高**：低半格那一次读把高半格锁存起来——先读高会拿到上一次的锁存值；
//! - **写闹钟先高后低**：低半格那一次写会**当场比较**，先写低半格时高半格还是旧值（首次为 0）
//!   ⇒ 会当场判成"到点了"。实测：按旧序写，第一次武装在目标之前约 99 millis 就报了一次。
//!
//! 它是**本域（RTC 驱动）的设备面**：那一页寄存器由本域从装配者手里领（`ONLY`）。

use runtime::core::dock::View;

/// 时间低 32 位（读它会锁存高半格）。
const TIME_LOW: usize = 0x00;
/// 时间高 32 位（上一次读低半格时锁存的）。
const TIME_HIGH: usize = 0x04;
/// 闹钟低半格（写它 = 当场比较一次）。
const ALARM_LOW: usize = 0x08;
/// 闹钟高半格。
const ALARM_HIGH: usize = 0x0c;
/// 中断使能（= 1 才许它拉线）。
const IRQ_ENABLED: usize = 0x10;
/// 撤闹钟。
const CLEAR_ALARM: usize = 0x14;
/// "闹钟武装着"（读它）。
const ALARM_STATUS: usize = 0x18;
/// 清 `irq_pending`。
const CLEAR_INTERRUPT: usize = 0x1c;

fn read(view: View, off: usize) -> u32 {
    // SAFETY: `view` 是 `Dock::open` 的产物——这一段已借映进本域；偏移落在 `reg` 区间内。
    unsafe { core::ptr::read_volatile((view.base() + off) as *const u32) }
}

fn write(view: View, off: usize, v: u32) {
    // SAFETY: 同上，只写这台设备的寄存器。
    unsafe { core::ptr::write_volatile((view.base() + off) as *mut u32, v) }
}

/// 现在几点（纳秒）。**先读低、再读高**——低半格那一次读会把高半格锁存起来，故那一对天生自洽。
pub fn now(view: View) -> u64 {
    let lo = read(view, TIME_LOW) as u64;
    let hi = read(view, TIME_HIGH) as u64;
    (hi << 32) | lo
}

/// 武装一次闹钟：`at`（纳秒）到点拉线。**先高后低**（见文件头），最后开闸。
pub fn arm(view: View, at: u64) {
    write(view, ALARM_HIGH, (at >> 32) as u32);
    write(view, ALARM_LOW, at as u32);
    write(view, IRQ_ENABLED, 1);
}

/// 闸门开着没有（读 `IRQ_ENABLED`）。
pub fn irq_enabled(view: View) -> u32 {
    read(view, IRQ_ENABLED)
}

/// 闹钟武装着没有（读 `ALARM_STATUS` = `alarm_running`）。
///
/// **照实记**：这一格**不是**"到点了"——响过之后 `alarm_running` 就归 0（见设备源码）。
/// 本域用它验"这一次真的被武装上了"，不拿它当"到点了"的判据。
pub fn armed(view: View) -> u32 {
    read(view, ALARM_STATUS)
}

/// 把到点那一格清掉：清 `irq_pending`（**电平源**，不清线就一直挂着）+ 撤闹钟。
///
/// 实测（探针）：同一段运行里不清是 **3093** 次投递，清掉是 **5** 次。
pub fn clear(view: View) {
    write(view, CLEAR_INTERRUPT, 1);
    write(view, CLEAR_ALARM, 1);
}
