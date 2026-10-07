//! 最小设备面：够三件事——读一次时间、武装下一次闹钟、把到点那一格清掉。
//! - **读时间先低后高**：低半格那一次读把高半格锁存起来——先读高会拿到上一次的锁存值；
//! - **写闹钟先高后低**：低半格那一次写会**当场比较**，先写低半格时高半格还是旧值（首次为 0）

use ::resource::dock::View;

/// 时间低 32 位（读它会锁存高半格）
const TIME_LOW: usize = 0x00;
/// 时间高 32 位（上一次读低半格时锁存的）
const TIME_HIGH: usize = 0x04;
/// 闹钟低半格（写它 = 当场比较一次）
const ALARM_LOW: usize = 0x08;
/// 闹钟高半格
const ALARM_HIGH: usize = 0x0c;
/// 中断使能（= 1 才许它拉线）
const IRQ_ENABLED: usize = 0x10;
/// 撤闹钟
const CLEAR_ALARM: usize = 0x14;
/// "闹钟武装着"（读它）
const ALARM_STATUS: usize = 0x18;
/// 清 `irq_pending`
const CLEAR_INTERRUPT: usize = 0x1c;

fn read(view: View, off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((view.base() + off) as *const u32) }
}

fn write(view: View, off: usize, v: u32) {
    // SAFETY: 同上，只写这台设备的寄存器。
    unsafe { core::ptr::write_volatile((view.base() + off) as *mut u32, v) }
}

/// 现在几点（纳秒）。**先读低、再读高**——低半格那一次读会把高半格锁存起来，故那一对天生自洽
pub fn now(view: View) -> u64 {
    let lo = read(view, TIME_LOW) as u64;
    let hi = read(view, TIME_HIGH) as u64;
    (hi << 32) | lo
}

/// 武装一次闹钟：`at`（纳秒）到点拉线。**先高后低**（见文件头），最后开闸
pub fn arm(view: View, at: u64) {
    write(view, ALARM_HIGH, (at >> 32) as u32);
    write(view, ALARM_LOW, at as u32);
    write(view, IRQ_ENABLED, 1);
}

/// 闸门开着没有（读 `IRQ_ENABLED`）
pub fn irq_enabled(view: View) -> u32 {
    read(view, IRQ_ENABLED)
}

/// 闹钟武装着没有（读 `ALARM_STATUS` = `alarm_running`）
pub fn armed(view: View) -> u32 {
    read(view, ALARM_STATUS)
}

/// 把到点那一格清掉：清 `irq_pending`（**电平源**，不清线就一直挂着）+ 撤闹钟
/// 实测（探针）：同一段运行里不清是 **3093** 次投递，清掉是 **5** 次
pub fn clear(view: View) {
    write(view, CLEAR_INTERRUPT, 1);
    write(view, CLEAR_ALARM, 1);
}
