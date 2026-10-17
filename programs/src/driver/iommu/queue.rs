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
/// IOFENCE.C 完成记录的基准值：完成记录里出现 `基准 ＋ 命令尾` 才算硬件执行过
/// **那一条**命令。
///
/// 固定值不行：上一次 `IOFENCE` 写下的那 4 字节会留在完成记录页里，下一次若只等
/// “看到某个固定值”，一枚迟到的旧存储就会冒充本次完成。故每次用**与本次命令尾绑定的**
/// 新值，并在提交前把那一格清零（旧值不留、旧存储也对不上新值）。
pub const FENCE_DATA: u32 = 0x10F0_0FEE;
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

/// 三个页都用**保留窗里的页号**指，构造时就从窗里取走这两组数：本域地址
/// （写命令项 / 读故障记录 / 读完成记录）与真实物理页起址（编程 CSR）。
///
/// 它**不持窗、也不持自己的尾**：命令尾每次都从 `CQT` 回读（硬件才知道消费到哪），
/// 故持有者可以各持一枚而不会分叉——产品那一侧 System 与它的平台各持一枚。
/// 等一条 `IOFENCE.C` 完成的上限（纳秒）。
const FENCE_NS: u64 = 2_000_000_000;
/// 等一台队列立起来 / 停下来与 `DDTP` 握手的上限（纳秒）。
const HANDSHAKE_NS: u64 = 2_000_000_000;

pub struct CommandQueue {
    base: usize,
    commands: usize,
    faults: usize,
    fence: usize,
    command_page: usize,
    fault_page: usize,
    fence_page: usize,
    /// 在途的那一条 `IOFENCE.C` 的完成值：重试只**继续等它**，不重提新命令。
    pending: Option<u32>,
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
            pending: None,
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

    /// 提交一条 `IOFENCE.C`（或**继续等**已在途的那一条）直到硬件写下本次的完成值。
    ///
    /// 完成值 = `FENCE_DATA + 提交时读到的命令尾`：每次提交一个新值（尾全局单调，
    /// 且提交由本域**唯一**串行提交者发出），故一枚迟到的旧完成存储对不上新值。
    /// 期限内没等到则返 `false` 并**保留那条在途命令**：下一次调用继续等它，
    /// 不重提新命令、不把已有的完成丢掉。
    pub fn fence(&mut self) -> bool {
        let cookie = match self.pending {
            Some(cookie) => cookie,
            None => {
                // 旧完成（含可能迟到的旧存储）不得冒充本次。
                // SAFETY: 完成记录页是本域刚取的可写页。
                unsafe { core::ptr::write_volatile(self.fence as *mut u32, 0) };
                super::order::sync();
                let tail = regs::read32(self.base, regs::CQT);
                let cookie = FENCE_DATA.wrapping_add(tail);
                let dword0 = IOFENCE_C
                    | IOFENCE_NOTIFY
                    | IOFENCE_PR
                    | IOFENCE_PW
                    | ((cookie as u64) << 32);
                let dword1 = (self.fence_page as u64) >> FENCE_ADDRESS_SHIFT;
                self.submit_at(tail, dword0, dword1);
                self.pending = Some(cookie);
                cookie
            }
        };
        let record = self.fence as *const u32;
        let until = env::chrono::clock().saturating_add(FENCE_NS);
        loop {
            // SAFETY: 完成记录页是本域刚取的可写页。
            let seen = unsafe { core::ptr::read_volatile(record) };
            if seen == cookie {
                self.pending = None;
                return true;
            }
            if env::chrono::clock() >= until {
                return false;
            }
        }
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
    fn submit(&mut self, dword0: u64, dword1: u64) {
        let tail = regs::read32(self.base, regs::CQT);
        self.submit_at(tail, dword0, dword1);
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

/// 有界轮询一台队列 CSR 直到 `done`（返最后那一刻的读数；到期限就把它原样交出去）。
fn poll(mut read: impl FnMut() -> u32, done: impl Fn(u32) -> bool) -> u32 {
    let mut seen = read();
    let until = env::chrono::clock().saturating_add(HANDSHAKE_NS);
    while !done(seen) && env::chrono::clock() < until {
        seen = read();
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
