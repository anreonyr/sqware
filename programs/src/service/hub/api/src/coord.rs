//! 绑定坐标：绑定号 → 各域都要用的那几个数（窗口记号、坐标孔记号、IOVA 窗起址）。
//!
//! 它们在 **Hub 那一本账的契约里**（不在某个驱动里）：协调入口与三个硬件域
//! 各自按同一个绑定号算出同一组坐标。记号只是**同一个绑定在不同表里**的同一个名字；
//! 权限与来源仍由内核逐枚核。

use env::Mark;

/// 每一号绑定给设备看到的那一段 IOVA 窗：`serial` 不复用，故窗也不复用。
pub const IOVA_BASE: u64 = 0x2_0000_0000;
/// 一段窗的长度上限（一页表级：`table::map_pages` 只铺同一条三级路径）。
pub const IOVA_SPAN: u64 = 1 << 21;
/// Exclusive end of the positive canonical Sv39 address range.
pub const IOVA_LIMIT: u64 = 1 << 38;
/// Exclusive serial limit derived from the non-reused IOVA windows.
pub const BINDING_MAX: u64 = (IOVA_LIMIT - IOVA_BASE) / IOVA_SPAN;

/// 记号族：绑定号 → 那一枚精确窗口（BAR）／给执行侧的 CPU 窗／给执行侧的坐标孔。
const BAR_MARK_BASE: u64 = 0x5043_4900_0000_0000;
const WINDOW_MARK_BASE: u64 = 0x5749_4e00_0000_0000;
const COORD_MARK_BASE: u64 = 0x434f_4f00_0000_0000;

/// 本次绑定那一枚精确窗口（BAR）在各域表里的记号。
pub const fn binding_mark(serial: u64) -> Mark {
    Mark::new(BAR_MARK_BASE + serial)
}

/// 本次绑定给执行侧（α）那一段 CPU 窗的记号。
pub const fn window_mark(serial: u64) -> Mark {
    Mark::new(WINDOW_MARK_BASE + serial)
}

/// Hub 放行业务时把设备坐标推给 α 那一枚孔的记号。
pub const fn coord_mark(serial: u64) -> Mark {
    Mark::new(COORD_MARK_BASE + serial)
}

/// 执行实例的构造方把「已构造的那一台」报给 Hub 的那一枚孔的记号。
///
/// 构造方（受限构造的客户）铸一枚孔、把**读端**借给 Hub、推一帧 {task}；
/// Hub 拿到的只是**一个号**，是否真是它名下、尚未放行的实例由 Control 那一面核
/// （`StateInstance` 的 owner 检查）：不靠报文自报。
pub const LAUNCH_MARK: Mark = Mark::of("hub-launch");

/// 本绑定的 IOVA 窗起址；`serial` 越界 ⇒ `None`（不猜一个回绕过的窗）。
pub const fn iova_base(serial: u64) -> Option<u64> {
    if serial == 0 || serial >= BINDING_MAX {
        return None;
    }
    Some(IOVA_BASE + serial * IOVA_SPAN)
}
