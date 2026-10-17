//! 命令队列与故障队列：提交命令、等硬件写回的完成记录、读故障记录。
//!
//! 队列是同步推进的（尾一写就处理），故「提交」= 写尾 + 等完成记录；
//! 完成记录那一页是硬件自己写的存储，它出现在保留窗里，且它是**在队列仍开着时**才能到的。
//!
//! 两台队列的开关都走规范那一手（同 Linux `iommu/riscv/iommu.c` 的
//! `riscv_iommu_queue_enable`/`_disable`）：写 CSR → **有界轮询到 busy 落** → 校验活动位。
//! 写后单次读不算走完协议。

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

/// 三个页都用**保留窗里的页号**指，构造时就从窗里取走这两组数：本域地址
/// （写命令项 / 读故障记录 / 读完成记录）与真实物理页起址（编程 CSR）。
///
/// 它**不持窗、也不持自己的尾**：命令尾每次都从 `CQT` 回读（硬件才知道消费到哪），
/// 故持有者可以各持一枚而不会分叉——产品那一侧 System 与它的平台各持一枚。
pub struct CommandQueue {
    base: usize,
    commands: usize,
    faults: usize,
    fence: usize,
    command_page: usize,
    fault_page: usize,
    fence_page: usize,
}

impl CommandQueue {
    pub fn new(
        base: usize,
        pages: &Pages,
        commands: usize,
        faults: usize,
        fence: usize,
    ) -> Self {
        Self {
            base,
            commands: pages.at(commands, 0) as usize,
            faults: pages.at(faults, 0) as usize,
            fence: pages.at(fence, 0) as usize,
            command_page: pages.physical(commands),
            fault_page: pages.physical(faults),
            fence_page: pages.physical(fence),
        }
    }

    /// 开两个队列：先故障队列（故障得有地方记），再命令队列。
    ///
    /// 每一台都走完协议：写 `ENABLE|MEM_FAULT`（顺手清掉粘住的内存故障位）→ 轮询到 busy 落
    /// → 校验 `ON` 真的立起来。**不开**中断位（本景轮询，没有 IOMMU 的中断向量）。
    pub fn enable(&mut self) -> Result<(), &'static str> {
        let faults =
            (regs::page_number(self.fault_page) << regs::PPN_SHIFT) | FAULT_LOG2SZ;
        regs::write64(self.base, regs::FQB, faults);
        let seen = self.raise(regs::FQCSR);
        if !active(seen, regs::QUEUE_ON) || seen & regs::QUEUE_MEM_FAULT != 0 {
            return Err("iommu: fault queue did not come up");
        }
        let commands = (regs::page_number(self.command_page) << regs::PPN_SHIFT) | COMMAND_LOG2SZ;
        regs::write64(self.base, regs::CQB, commands);
        let seen = self.raise(regs::CQCSR);
        if !active(seen, regs::QUEUE_ON) || seen & regs::QUEUE_MEM_FAULT != 0 {
            return Err("iommu: command queue did not come up");
        }
        Ok(())
    }

    /// 落两个队列（先命令、后故障）：写 0 → 轮询到 busy 落；返最后那一刻的真实读数。
    ///
    /// 「停」与「忙已清」仍然是两件事，故判定在外面（要的是各自那一对读数）。
    pub fn disable(&mut self) -> (u32, u32) {
        let commands = self.lower(regs::CQCSR);
        let faults = self.lower(regs::FQCSR);
        (commands, faults)
    }

    /// 抬一台队列并等它真的立起来：写 `ENABLE|MEM_FAULT`（清粘住的故障），轮询 busy 落。
    fn raise(&self, csr: usize) -> u32 {
        regs::write32(self.base, csr, regs::QUEUE_ENABLE | regs::QUEUE_MEM_FAULT);
        poll(
            || regs::read32(self.base, csr),
            |seen| seen & regs::QUEUE_BUSY == 0,
        )
    }

    /// 落一台队列并等它真的停下来（同样有界）。
    fn lower(&self, csr: usize) -> u32 {
        regs::write32(self.base, csr, 0);
        poll(
            || regs::read32(self.base, csr),
            |seen| seen & regs::QUEUE_BUSY == 0,
        )
    }

    /// 让这个 IOMMU 忘掉此前翻译出来的那些项。
    pub fn invalidate_all(&mut self) {
        self.submit(IOTINVAL_VMA, 0);
    }

    /// 提交一条 IOFENCE.C 并等它写下的完成记录；`false` = 期限内没等到。
    pub fn fence(&mut self) -> bool {
        let dword0 = IOFENCE_C | IOFENCE_NOTIFY | ((FENCE_DATA as u64) << 32);
        let dword1 = (self.fence_page as u64) >> FENCE_ADDRESS_SHIFT;
        self.submit(dword0, dword1);
        let record = self.fence as *const u32;
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

    /// 硬件记下的故障条数与最近一条的 `(cause, devid, ttype)`（0 条时报全 0）。
    ///
    /// 一条 32 字节记录的头 8 字节：cause[11:0] / pid[31:12] / pv[32] /
    /// ttype[39:34] / **did[63:40]** —— 谁被拒、拒在哪一步，都在这一个数里。
    pub fn faults(&self) -> (u32, u32, u32, u32) {
        let count = regs::read32(self.base, regs::FQT) & (FAULT_ENTRIES - 1);
        if count == 0 {
            return (0, 0, 0, 0);
        }
        let slot = ((count - 1) % FAULT_ENTRIES) as usize * FAULT_BYTES;
        // SAFETY: 故障队列页是本域刚取的可写页，记录按 32 字节排。
        let header = unsafe { core::ptr::read_volatile((self.faults + slot) as *const u64) };
        (
            count,
            (header & 0xFFF) as u32,
            (header >> 40) as u32,
            ((header >> 34) & 0x3F) as u32,
        )
    }

    /// 提交一项：先回读命令尾，再把那一项写在当前槽上，最后推门铃。
    ///
    /// 尾取自 `CQT`（硬件消费到哪它才知道），故本层不藏自己的计数；两次提交之间
    /// 没有别的写者时就与“本地自增”同一条结果。
    fn submit(&mut self, dword0: u64, dword1: u64) {
        let tail = regs::read32(self.base, regs::CQT);
        let slot = (tail % COMMAND_ENTRIES) as usize * COMMAND_BYTES;
        let at = (self.commands + slot) as *mut u64;
        // SAFETY: 命令队列页是本域刚取的可写页，项按 16 字节排。
        unsafe {
            core::ptr::write_volatile(at, dword0);
            core::ptr::write_volatile(at.add(1), dword1);
        }
        // 命令字与门铃之间要显式有序：设备那一侧才看得见刚写下那一项。
        super::order::sync();
        regs::write32(self.base, regs::CQT, tail.wrapping_add(1));
    }
}

/// 有界轮询一台队列 CSR 直到 `done`（返最后那一刻的读数；到期限就把它原样交出去）。
fn poll(mut read: impl FnMut() -> u32, done: impl Fn(u32) -> bool) -> u32 {
    let mut seen = read();
    let mut left = POLL_LIMIT;
    while !done(seen) && left > 0 {
        seen = read();
        left -= 1;
    }
    seen
}

/// 队列已活动且不忙。
pub fn active(csr: u32, on: u32) -> bool {
    csr & on != 0 && csr & regs::QUEUE_BUSY == 0
}

/// 队列已停：活动位落了、忙也清了。
pub fn quiet(csr: u32, on: u32) -> bool {
    csr & on == 0 && csr & regs::QUEUE_BUSY == 0
}

/// 命令队列自己的错：内存故障 / 命令超时 / 非法命令。
///
/// `CQCSR[9]` 是 `CMD_TO`、`[10]` 是 `CMD_ILL`——两个名字与故障/页请求队列的溢出位**不同一件事**，
/// 故两者不共用一条判据。
pub fn command_error(csr: u32) -> bool {
    csr & (regs::QUEUE_MEM_FAULT | regs::QUEUE_CMD_TO | regs::QUEUE_CMD_ILL) != 0
}

/// 故障队列 / 页请求队列自己的错：内存故障 / 溢出。
pub fn queue_error(csr: u32) -> bool {
    csr & (regs::QUEUE_MEM_FAULT | regs::QUEUE_OVERFLOW) != 0
}
