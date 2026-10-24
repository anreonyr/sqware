//! 命令队列与故障队列：提交命令、等硬件写回的完成记录、读故障记录。
//!
//! 队列是同步推进的（尾一写就处理），故「提交」= 写尾 + 等完成记录；
//! 完成记录那一页是硬件自己写的存储，它出现在保留窗里，且它是**在队列仍开着时**才能到的。
//!
//! 两台队列的开关都走规范那一手（同 Linux `iommu/riscv/iommu.c` 的
//! `riscv_iommu_queue_enable`/`_disable`）：写 CSR → **有界轮询到 busy 落** → 校验活动位。
//! 写后单次读不算走完协议。
//!
//! **两档入口，同一套机制**：
//! - `enable`/`disable`/`fence` 是**同步**那一档（探针/引导：等完再往下走）；
//! - `raise`/`lower`/`fence_step` 是**一步**那一档（System 的帧：写只写一次，回读
//!   每帧一次，没等到就把 [`regs::Handshake`] 与在途的那条命令原样留到下一帧）。
//! 两档共用同一次写入与同一条 `busy` 判据，故不是两套协议。

use super::fence::Fence;
use super::handshake::Handshake;
use super::pages::Pages;
use super::regs;
use super::ticket::Ticket;

/// 命令项 16 字节；故障记录 32 字节。
const COMMAND_BYTES: usize = 16;
const FAULT_BYTES: usize = 32;
/// 一页队列的长度字段：条目数 = 2 << log2sz。
const COMMAND_LOG2SZ: u64 = 7;
const FAULT_LOG2SZ: u64 = 6;
const COMMAND_ENTRIES: u32 = 2 << COMMAND_LOG2SZ;
const FAULT_ENTRIES: u32 = 2 << FAULT_LOG2SZ;
/// First completion value; ticket values increase without wrapping.
pub const FENCE_DATA: u32 = 0x10F0_0FEE;
/// Completion slots available in one retained page.
pub const FENCE_SLOTS: usize = 1024;
/// 完成记录地址在命令里按 4 字节格给（地址 >> 2）。
const FENCE_ADDRESS_SHIFT: u32 = 2;
/// 命令操作码：IOTINVAL（功能 VMA）/ IOFENCE（功能 C）。
const IOTINVAL_VMA: u64 = 1;
const IOFENCE_C: u64 = 2;
/// `IOFENCE.C` 那几个位：AV[10] 带完成记录、PR[12] 等已发出的读、PW[13] 等已发出的写。
///
/// 编码照规范（`opcode[6:0] func3[9:7] AV[10] WSI[11] PR[12] PW[13] rsvd[31:14] DATA[63:32]`），
/// 但**不把“完成记录出现”当成端点在途已排空**：本机 QEMU 把 `PR`/`PW` 当保留位忽略
/// （`riscv_iommu_iofence` 处理命令时就立刻写记录），而硬件即使支持也只保证“已被
/// IOMMU 处理过的请求”落全局序。端点在途那一段由端点侧收口（清 bus-master）单独管。
const IOFENCE_NOTIFY: u64 = 1 << 10;
const IOFENCE_PR: u64 = 1 << 12;
const IOFENCE_PW: u64 = 1 << 13;

/// 等一条 `IOFENCE.C` 完成的上限（纳秒）。
const FENCE_NS: u64 = 2_000_000_000;
/// 等一台队列立起来 / 停下来与 `DDTP` 握手的上限（纳秒）。
const HANDSHAKE_NS: u64 = 2_000_000_000;

/// The single submitter for the shared command and fault queues.
pub struct CommandQueue {
    base: usize,
    commands: usize,
    faults: usize,
    fence: usize,
    command_page: usize,
    fault_page: usize,
    fence_page: usize,
    next_cookie: Option<u32>,
}

impl CommandQueue {
    pub fn new(base: usize, pages: &Pages, commands: usize, faults: usize, fence: usize) -> Self {
        Self {
            base,
            commands: pages.at(commands, 0) as usize,
            faults: pages.at(faults, 0) as usize,
            fence: pages.at(fence, 0) as usize,
            command_page: pages.physical(commands),
            fault_page: pages.physical(faults),
            fence_page: pages.physical(fence),
            next_cookie: Some(FENCE_DATA),
        }
    }

    /// 这一枚队列所在的寄存器窗起址。
    pub fn base(&self) -> usize {
        self.base
    }

    /// 开两个队列：先故障队列（故障得有地方记），再命令队列。
    ///
    /// 每一台都走完协议：写 `ENABLE|MEM_FAULT`（顺手清掉粘住的内存故障位）→ 轮询到 busy 落
    /// → 校验 `ON` 真的立起来。**不开**中断位（本景轮询，没有 IOMMU 的中断向量）。
    pub fn enable(&mut self) -> Result<(), &'static str> {
        let faults = self.raise(regs::FQCSR).wait(
            || regs::read32(self.base, regs::FQCSR),
            |seen| seen & regs::QUEUE_BUSY == 0,
        );
        if !active(faults, regs::QUEUE_ON) || faults & regs::QUEUE_MEM_FAULT != 0 {
            return Err("iommu: fault queue did not come up");
        }
        let commands = self.raise(regs::CQCSR).wait(
            || regs::read32(self.base, regs::CQCSR),
            |seen| seen & regs::QUEUE_BUSY == 0,
        );
        if !active(commands, regs::QUEUE_ON) || commands & regs::QUEUE_MEM_FAULT != 0 {
            return Err("iommu: command queue did not come up");
        }
        Ok(())
    }

    /// 抬一台队列的第一步：写它的基址、写 `ENABLE|MEM_FAULT`（顺手清掉粘住的内存故障位），
    /// 并起一次握手。之后自己按帧用 [`regs::read32`] **单次读**它落没落。
    ///
    /// `csr` 只认故障队列或命令队列：两台各自写自己那一格基址与那一档长度。
    pub fn raise(&self, csr: usize) -> Handshake {
        let (base_csr, page, log2sz) = if csr == regs::FQCSR {
            (regs::FQB, self.fault_page, FAULT_LOG2SZ)
        } else {
            (regs::CQB, self.command_page, COMMAND_LOG2SZ)
        };
        let value = (regs::page_number(page) << regs::PPN_SHIFT) | log2sz;
        regs::write64(self.base, base_csr, value);
        regs::write32(self.base, csr, regs::QUEUE_ENABLE | regs::QUEUE_MEM_FAULT);
        Handshake::new(HANDSHAKE_NS)
    }

    /// 落两台队列（先命令、后故障）：写 0 → 轮询到 busy 落；返最后那一刻的真实读数。
    ///
    /// 「停」与「忙已清」仍然是两件事，故判定在外面（要的是各自那一对读数）。
    pub fn disable(&mut self) -> (u32, u32) {
        let commands = self.lower(regs::CQCSR).wait(
            || regs::read32(self.base, regs::CQCSR),
            |seen| seen & regs::QUEUE_BUSY == 0,
        );
        let faults = self.lower(regs::FQCSR).wait(
            || regs::read32(self.base, regs::FQCSR),
            |seen| seen & regs::QUEUE_BUSY == 0,
        );
        (commands, faults)
    }

    /// 落一台队列的第一步：写 0 并起一次握手（与 [`CommandQueue::raise`] 对称，只是要落的数不同）。
    pub fn lower(&self, csr: usize) -> Handshake {
        regs::write32(self.base, csr, 0);
        Handshake::new(HANDSHAKE_NS)
    }

    /// 让这个 IOMMU 忘掉此前翻译出来的那些项。
    pub fn invalidate_all(&mut self) -> bool {
        self.submit(IOTINVAL_VMA, 0)
    }

    /// Reserve a publish/retire pair before acquiring a mapping's frame reference.
    /// The caller owns the slots until both commands have completed.
    pub fn tickets(&mut self, publish: usize, retire: usize) -> Option<(Ticket, Ticket)> {
        if publish >= FENCE_SLOTS || retire >= FENCE_SLOTS || publish == retire {
            return None;
        }
        let first = super::cookie::reserve(&mut self.next_cookie, 2)?;
        let make = |slot, cookie| Ticket {
            queue: self.fence,
            slot: slot * 4,
            cookie,
            handshake: None,
        };
        Some((make(publish, first), make(retire, first + 1)))
    }

    /// Wait on this ticket without consuming another operation's completion.
    pub fn fence(&mut self, ticket: &mut Ticket) -> bool {
        let capacity_wait = Handshake::new(FENCE_NS);
        loop {
            match self.fence_step(ticket) {
                Fence::Settled => return true,
                Fence::Expired => return false,
                Fence::Waiting => {}
            }
            if ticket.handshake.is_none() && capacity_wait.expired() {
                return false;
            }
        }
    }

    /// Submit once, then read the same real completion under the same deadline.
    pub fn fence_step(&mut self, ticket: &mut Ticket) -> Fence {
        if ticket.queue != self.fence {
            return Fence::Expired;
        }
        if ticket.handshake.is_none() {
            if self.full() {
                return Fence::Waiting;
            }
            // SAFETY: tickets() bounds the slot to the retained completion page.
            unsafe { core::ptr::write_volatile((self.fence + ticket.slot) as *mut u32, 0) };
            super::order::sync();
            let dword0 =
                IOFENCE_C | IOFENCE_NOTIFY | IOFENCE_PR | IOFENCE_PW | ((ticket.cookie as u64) << 32);
            let dword1 = ((self.fence_page + ticket.slot) as u64) >> FENCE_ADDRESS_SHIFT;
            if !self.submit(dword0, dword1) {
                return Fence::Waiting;
            }
            ticket.handshake = Some(Handshake::new(FENCE_NS));
        }
        let (record, cookie) = ticket.completion();
        // SAFETY: 完成记录页是本域刚取的可写页。
        let seen = unsafe { core::ptr::read_volatile(record) };
        if seen == cookie {
            return Fence::Settled;
        }
        if ticket.handshake.as_ref().expect("submitted ticket").expired() {
            return Fence::Expired;
        }
        Fence::Waiting
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
    /// 尾取自 `CQT`（硬件消费到哪它才知道），故本层不藏自己的计数。**这一读一写
    /// 不是并发互斥**：同一时刻只应有一个提交者（本域就是 System 自己的执行权），
    /// 多个句柄靠回读尾只保证“不藏分叉的本地计数”，不保证两个提交者不撞槽。
    fn full(&self) -> bool {
        let tail = regs::read32(self.base, regs::CQT);
        let head = regs::read32(self.base, regs::CQH);
        (tail.wrapping_add(1) % COMMAND_ENTRIES) == (head % COMMAND_ENTRIES)
    }

    fn submit(&mut self, dword0: u64, dword1: u64) -> bool {
        if self.full() {
            return false;
        }
        let tail = regs::read32(self.base, regs::CQT);
        self.submit_at(tail, dword0, dword1);
        true
    }

    /// 在指定尾上写一项并推门铃（调用方已经拿过尾，故可与它写下的完成值绑定）。
    fn submit_at(&mut self, tail: u32, dword0: u64, dword1: u64) {
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
