//! 本域那一侧的停机凭据：`Holds` 那本账在放下帧引用之前问的那一问。
//!
//! 它只**回读**本域已经做过的停链（Hub 按合同顺序推进各域：撤翻译 → 清 MASTER →
//! 设备 reset/drain → 关队列），不代别的域做活、也不解释别人的寄存器：
//! 目录项已清、两台队列已停（含页请求队列从未开）——两格都对上才算本域已停。
//! 没对上就答 `Pending`：帧引用原样留着，下一格再核。

use crate::driver::iommu::hold::{DeviceRange, Failure, Platform};
use crate::driver::iommu::queue;
use crate::driver::iommu::regs;

pub struct Settled {
    range: DeviceRange,
    /// 寄存器窗在本域的起址。
    base: usize,
    /// 本域那一页设备目录表（DDT）的起址；活到本对象最后一次访问之后。
    ddt: *const u64,
    requester: u16,
}

impl Settled {
    /// `ddt` 是本域那一页目录表的首字指针（页与本对象同寿命）。
    pub fn new(range: DeviceRange, base: usize, ddt: *const u64, requester: u16) -> Self {
        Self {
            range,
            base,
            ddt,
            requester,
        }
    }

    /// 本域那一侧的停链回读。
    fn settled(&self) -> Result<(), Failure> {
        // ① 本次绑定那一格目录项已清（撤翻译的凭据）。
        let at = unsafe { self.ddt.add(self.requester as usize * 8) };
        for index in 0..8 {
            // SAFETY: 目录表那一页在本域已映射，`requester` 由本次绑定核过。
            let word = unsafe { core::ptr::read_volatile(at.add(index)) };
            if word != 0 {
                return Err(Failure::Pending);
            }
        }
        // ② 两台队列已停（关队列的凭据）。
        let commands = regs::read32(self.base, regs::CQCSR);
        let faults = regs::read32(self.base, regs::FQCSR);
        if !queue::quiet(commands, regs::QUEUE_ON) || !queue::quiet(faults, regs::QUEUE_ON) {
            return Err(Failure::Pending);
        }
        Ok(())
    }
}

impl Platform for Settled {
    fn range(&self) -> DeviceRange {
        self.range
    }

    /// 正常收场与故障收场走同一条回读（释放前的凭据不更低）。
    fn synchronize(&mut self) -> Result<(), Failure> {
        self.settled()
    }

    fn isolate(&mut self) -> Result<(), Failure> {
        self.settled()
    }
}
