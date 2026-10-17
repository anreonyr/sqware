//! 命令队列与故障队列：提交命令、等硬件写回的完成记录、读故障记录。
//!
//! 队列是同步推进的（尾一写就处理），故「提交」= 写尾 + 等完成记录；
//! 完成记录那一页是硬件自己写的存储，它出现在保留窗里，且它是**在队列仍开着时**才能到的。

use super::pages::Pages;
use super::regs;

/// 命令项 16 字节；故障记录 32 字节。
const COMMAND_BYTES: usize = 16;
const FAULT_BYTES: usize = 32;
/// 一页队列的长度字段：条目数 = 2 << log2sz。
const COMMAND_LOG2SZ: u64 = 7;
const FAULT_LOG2SZ: u64 = 6;
const COMMAND_ENTRIES: u32 = 2 << COMMAND_LOG2SZ;
const FAULT_ENTRIES: u32 = 2 << FAULT_LOG2SZ;
/// IOFENCE.C 写下的那个数：完成记录里出现它，才算硬件执行过这条命令。
pub const FENCE_DATA: u32 = 0x10F0_0FEE;
/// 完成记录地址在命令里按 4 字节格给（地址 >> 2）。
const FENCE_ADDRESS_SHIFT: u32 = 2;
/// 命令操作码：IOTINVAL（功能 VMA）/ IOFENCE（功能 C）。
const IOTINVAL_VMA: u64 = 1;
const IOFENCE_C: u64 = 2;
/// IOFENCE.C 的 AV：带完成记录。
const IOFENCE_NOTIFY: u64 = 1 << 10;
/// 等完成记录的有界自旋。
const POLL_LIMIT: usize = 1_000_000;

pub struct CommandQueue<'a> {
    base: usize,
    pages: &'a Pages,
    commands: usize,
    faults: usize,
    fence: usize,
    tail: u32,
}

impl<'a> CommandQueue<'a> {
    /// 三个页都用**保留窗里的页号**指：窗同时知道本域地址与真实物理起址，
    /// 于是写命令用前者、编程给设备用后者，不会把两者弄混。
    pub fn new(base: usize, pages: &'a Pages, commands: usize, faults: usize, fence: usize) -> Self {
        Self {
            base,
            pages,
            commands,
            faults,
            fence,
            tail: 0,
        }
    }

    /// 开两个队列：先故障队列（故障得有地方记），再命令队列；各自确认「活动且不忙」。
    pub fn enable(&mut self) -> Result<(), &'static str> {
        let faults = (regs::page_number(self.pages.physical(self.faults)) << regs::PPN_SHIFT)
            | FAULT_LOG2SZ;
        regs::write64(self.base, regs::FQB, faults);
        regs::write32(self.base, regs::FQCSR, regs::QUEUE_ENABLE);
        if !active(regs::read32(self.base, regs::FQCSR), regs::QUEUE_ON) {
            return Err("iommu-probe: fault queue did not come up");
        }
        let commands = (regs::page_number(self.pages.physical(self.commands)) << regs::PPN_SHIFT)
            | COMMAND_LOG2SZ;
        regs::write64(self.base, regs::CQB, commands);
        regs::write32(self.base, regs::CQCSR, regs::QUEUE_ENABLE);
        if !active(regs::read32(self.base, regs::CQCSR), regs::QUEUE_ON) {
            return Err("iommu-probe: command queue did not come up");
        }
        Ok(())
    }

    /// 撤下两个队列，返回写完之后各自的真实读数（判定在外面，现场带数）。
    pub fn disable(&mut self) -> (u32, u32) {
        regs::write32(self.base, regs::CQCSR, 0);
        let commands = regs::read32(self.base, regs::CQCSR);
        regs::write32(self.base, regs::FQCSR, 0);
        let faults = regs::read32(self.base, regs::FQCSR);
        (commands, faults)
    }

    /// 让这个 IOMMU 忘掉此前翻译出来的那些项。
    pub fn invalidate_all(&mut self) {
        self.submit(IOTINVAL_VMA, 0);
    }

    /// 提交一条 IOFENCE.C 并等它写下的完成记录；`false` = 期限内没等到。
    pub fn fence(&mut self) -> bool {
        let dword0 = IOFENCE_C | IOFENCE_NOTIFY | ((FENCE_DATA as u64) << 32);
        let dword1 = (self.pages.physical(self.fence) as u64) >> FENCE_ADDRESS_SHIFT;
        self.submit(dword0, dword1);
        let record = self.pages.at(self.fence, 0) as *const u32;
        for _ in 0..POLL_LIMIT {
            // SAFETY: 完成记录页是本域刚取的可写页。
            let seen = unsafe { core::ptr::read_volatile(record) };
            if seen == FENCE_DATA {
                return true;
            }
        }
        false
    }

    /// 硬件消费到哪了：两个队列的头。
    pub fn heads(&self) -> (u32, u32) {
        (
            regs::read32(self.base, regs::CQH),
            regs::read32(self.base, regs::FQH),
        )
    }

    /// 两个队列控制寄存器的当前原值（含故障与非法命令位）。
    pub fn status(&self) -> (u32, u32) {
        (
            regs::read32(self.base, regs::CQCSR),
            regs::read32(self.base, regs::FQCSR),
        )
    }

    /// 两个队列基址寄存器里真正落下的页号（与保留窗里那两页对照）。
    pub fn bases(&self) -> (u64, u64) {
        (
            (regs::read64(self.base, regs::CQB) & regs::PPN_FIELD) >> regs::PPN_SHIFT,
            (regs::read64(self.base, regs::FQB) & regs::PPN_FIELD) >> regs::PPN_SHIFT,
        )
    }

    /// 硬件记下的故障条数与最近一条的原因码（0 = 没有故障）。
    pub fn faults(&self) -> (u32, u32) {
        let count = regs::read32(self.base, regs::FQT) & (FAULT_ENTRIES - 1);
        if count == 0 {
            return (0, 0);
        }
        let slot = ((count - 1) % FAULT_ENTRIES) as usize * FAULT_BYTES;
        // SAFETY: 故障队列页是本域刚取的可写页，记录按 32 字节排。
        let header = unsafe { core::ptr::read_volatile(self.pages.at(self.faults, slot) as *const u64) };
        (count, (header & 0xFFF) as u32)
    }

    fn submit(&mut self, dword0: u64, dword1: u64) {
        let slot = (self.tail % COMMAND_ENTRIES) as usize * COMMAND_BYTES;
        let at = self.pages.at(self.commands, slot) as *mut u64;
        // SAFETY: 命令队列页是本域刚取的可写页，项按 16 字节排。
        unsafe {
            core::ptr::write_volatile(at, dword0);
            core::ptr::write_volatile(at.add(1), dword1);
        }
        self.tail = self.tail.wrapping_add(1);
        regs::write32(self.base, regs::CQT, self.tail);
    }
}

/// 队列已活动且不忙。
pub fn active(csr: u32, on: u32) -> bool {
    csr & on != 0 && csr & regs::QUEUE_BUSY == 0
}

/// 队列已停：活动位落了、忙也清了。
pub fn quiet(csr: u32, on: u32) -> bool {
    csr & on == 0 && csr & regs::QUEUE_BUSY == 0
}

/// 队列自己报的内存故障 / 溢出位。
pub fn queue_error(csr: u32) -> bool {
    csr & (regs::QUEUE_MEM_FAULT | regs::QUEUE_OVERFLOW) != 0
}
