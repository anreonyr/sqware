//! 绑定交付所用的资源标记；权限、来源与完整绑定身份另行核验。

use env::Mark;

/// 记号族：绑定号 → 那一枚精确窗口（BAR）／给执行侧的 CPU 窗／给执行侧的坐标孔。
const BAR_MARK_BASE: u64 = 0x5043_4900_0000_0000;
const WINDOW_MARK_BASE: u64 = 0x5749_4e00_0000_0000;
const COORD_MARK_BASE: u64 = 0x434f_4f00_0000_0000;

/// 本次绑定那一枚精确窗口（BAR）在各域表里的记号。
pub const fn binding_mark(serial: u64) -> Mark {
    Mark::new(BAR_MARK_BASE.wrapping_add(serial))
}

/// 本次绑定给执行侧（α）那一段 CPU 窗的记号。
pub const fn window_mark(serial: u64) -> Mark {
    Mark::new(WINDOW_MARK_BASE.wrapping_add(serial))
}

/// Hub 放行业务时把设备坐标推给 α 那一枚孔的记号。
pub const fn coord_mark(serial: u64) -> Mark {
    Mark::new(COORD_MARK_BASE.wrapping_add(serial))
}

/// 执行实例的构造方把「已构造的那一台」报给 Hub 的那一枚孔的记号。
///
/// 构造方（受限构造的客户）铸一枚孔、把**读端**借给 Hub、推一帧 {task}；
/// Hub 拿到的只是**一个号**，是否真是它名下、尚未放行的实例由 Control 那一面核
/// （`StateInstance` 的 owner 检查）：不靠报文自报。
pub const LAUNCH_MARK: Mark = Mark::of("hub-launch");
