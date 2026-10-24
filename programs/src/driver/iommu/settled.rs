//! 本域那一侧的停机凭据：`Holds` 那本账在放下帧引用之前问的那一问。
//!
//! 回读目录项、实际撤销完成值和队列静止状态。
//! 任一证明不足就保留帧引用，等待同一责任重试。

use crate::driver::iommu::hold::{DeviceRange, Failure, Platform};
use crate::driver::iommu::context;
use crate::driver::iommu::queue;
use crate::driver::iommu::regs;
use crate::driver::iommu::ticket::Ticket;

pub struct Settled {
    range: DeviceRange,
    /// 寄存器窗在本域的起址。
    base: usize,
    /// 本域那一页设备目录表（DDT）的起址；活到本对象最后一次访问之后。
    ddt: *const u64,
    requester: u16,
    retire: *const u32,
    cookie: u32,
}

impl Settled {
    /// `ddt` 是本域那一页目录表的首字指针（页与本对象同寿命）。
    pub fn new(range: DeviceRange, base: usize, ddt: *const u64, requester: u16, retire: &Ticket) -> Self {
        let (retire, cookie) = retire.completion();
        Self {
            range,
            base,
            ddt,
            requester,
            retire,
            cookie,
        }
    }

    /// 本域那一侧的停链回读。
    fn settled(&self) -> Result<(), Failure> {
        crate::driver::iommu::order::sync();
        // ① 本次绑定那一格目录项已清（撤翻译的凭据）。
        let offset = context::offset(self.requester).ok_or(Failure::Denied)?;
        let at = unsafe { self.ddt.add(offset / core::mem::size_of::<u64>()) };
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
        let requests = regs::read32(self.base, regs::PQCSR);
        if ![commands, faults, requests].into_iter().all(|csr|
            queue::quiet(csr, regs::QUEUE_ON) && csr & regs::QUEUE_ENABLE == 0 && !queue::queue_error(csr)
        ) || queue::command_error(commands) {
            return Err(Failure::Pending);
        }
        // SAFETY: this immutable slot belongs to the retained pool until the scope closes.
        let completed = unsafe { core::ptr::read_volatile(self.retire) } == self.cookie;
        if !completed { return Err(Failure::Pending); }
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
