//! 本设备的常驻 BAR 恢复映射、能力表布局与当前执行绑定。

use env::TaskId;

use crate::driver::pci::virtio::Caps;

pub struct Device {
    pub serial: u64,
    pub execution: TaskId,
    pub pages: u32,
    pub caps: Caps,
    /// reset/drain 做过了吗（重试不重做）。
    pub stopped: bool,
    pub granted: bool,
}
