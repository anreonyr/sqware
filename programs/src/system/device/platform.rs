//! 一枚真实 RISC-V IOMMU 的**独立停机权威**：System 用来隔断外访存的入口。
//!
//! 它把 [`Platform`] 两个入口落到真寄存器上：
//! - `synchronize`（正常关闭）：`IOTINVAL` ＋ `IOFENCE.C`，等硬件写回的完成记录
//!   ——翻译撤销、设备缓存排空**都收在那一枚完成记录里**；
//! - `isolate`（故障关闭）：先把在途命令排空（同一条完成记录），再把设备目录表
//!   `DDTP` 落 Off 并回读确认 mode 与 busy，最后按握手停两台队列并各自确认
//!   `on` 落了、`busy` 也清了。
//!
//! 三次“等”都是**有界进展步骤**：期限内没等到就返 `Pending`，引用原样保留、下次
//! 接着来。CPU 图撤了、task 退了、只写一个布尔——都不构成这里的确认。
//!
//! 它不向设备服务要任何东西：命令队列与故障队列的那些页在本域手里，寄存器写也从
//! 本域发出，故“服务死掉”不改它能不能停。

use crate::driver::iommu::{
    queue::{self, CommandQueue},
    regs,
};
use crate::system::supply::retention::{DeviceRange, Failure, Platform};

/// System 里那一枚 IOMMU 的停机权威：一段设备范围 ＋ 一枚随时可发命令的队列。
pub struct IommuPlatform {
    range: DeviceRange,
    /// 寄存器窗在本域的起址（已授权、已映射）。
    base: usize,
    /// 命令/故障/完成记录那三页（本域地址与真实物理页起址都在里面）。
    queue: CommandQueue,
}

impl IommuPlatform {
    /// `queue` 由管理者在本域建好（那时它才拿得到那三页的真实物理页起址）。
    pub fn new(range: DeviceRange, base: usize, queue: CommandQueue) -> Self {
        Self { range, base, queue }
    }
}

impl Platform for IommuPlatform {
    fn range(&self) -> DeviceRange {
        self.range
    }

    /// 正常关闭：翻译撤销 ＋ 设备缓存排空，以 `IOFENCE.C` 的完成记录为准。
    ///
    /// 完成记录写不下就返 `Pending`（在途存储还没排空，不能放页），引用保留重试。
    fn synchronize(&mut self) -> Result<(), Failure> {
        self.queue.invalidate_all();
        if !self.queue.fence() {
            return Err(Failure::Pending);
        }
        Ok(())
    }

    /// 故障关闭：System 自己把整个范围停住并确认，不依赖那一头还活着。
    ///
    /// 四步各自确认：在途命令排空（完成记录）→ 目录表落 Off（回读 mode/busy）
    /// → 两台队列停（回读 on/busy）→ 没有队列自己的错、页请求队列从未启用。
    fn isolate(&mut self) -> Result<(), Failure> {
        self.queue.invalidate_all();
        if !self.queue.fence() {
            return Err(Failure::Pending);
        }
        let stopped = match regs::ddtp_set(self.base, regs::DDTP_MODE_OFF, None) {
            Ok(seen) => seen,
            Err(_) => return Err(Failure::Pending),
        };
        if stopped & 0xF != regs::DDTP_MODE_OFF || stopped & regs::DDTP_BUSY != 0 {
            return Err(Failure::Unavailable);
        }
        let (commands, faults) = self.queue.disable();
        let page_request = regs::read32(self.base, regs::PQCSR);
        let stopped_queues = queue::quiet(commands, regs::QUEUE_ON)
            && queue::quiet(faults, regs::QUEUE_ON)
            && page_request & (regs::QUEUE_ENABLE | regs::QUEUE_ON) == 0;
        let clean = !queue::command_error(commands) && !queue::queue_error(faults);
        if !stopped_queues || !clean {
            return Err(Failure::Unavailable);
        }
        Ok(())
    }
}
