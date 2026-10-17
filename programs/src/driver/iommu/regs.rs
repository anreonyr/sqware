//! 设备面：RISC-V IOMMU 的寄存器格。
//!
//! 偏移与位取自 RISC-V IOMMU 规范 1.0 的寄存器表，并与 QEMU
//! `hw/riscv/riscv-iommu-bits.h` 逐项核对过；本层只说「第几格、哪一位、怎么读写」，
//! 队列与上下文的语义在各自那一层。

/// 能力寄存器（64 位）：版本、支持的地址模式与 PAS 位宽。
pub const CAP: usize = 0x0000;
/// 设备目录表指针（64 位）：mode[3:0] | busy[4] | ppn[53:10]。
pub const DDTP: usize = 0x0010;
/// 命令队列基址（64 位）：log2sz[4:0] | ppn[53:10]。
pub const CQB: usize = 0x0018;
/// 命令队列头 / 尾（32 位）。
pub const CQH: usize = 0x0020;
pub const CQT: usize = 0x0024;
/// 故障队列基址、头、尾。
pub const FQB: usize = 0x0028;
pub const FQH: usize = 0x0030;
pub const FQT: usize = 0x0034;
/// 三个队列的控制 / 状态寄存器（32 位）。
pub const CQCSR: usize = 0x0048;
pub const FQCSR: usize = 0x004C;
pub const PQCSR: usize = 0x0050;
/// 翻译请求（调试口）三格：请求地址、控制、响应。
pub const TR_REQ_IOVA: usize = 0x0258;
pub const TR_REQ_CTL: usize = 0x0260;
pub const TR_RESPONSE: usize = 0x0268;

/// 队列 CSR 位：enable[0] / 中断使能[1] / 内存故障[8] / 溢出[9] / 活动[16] / 忙[17]。
pub const QUEUE_ENABLE: u32 = 1 << 0;
pub const QUEUE_MEM_FAULT: u32 = 1 << 8;
pub const QUEUE_OVERFLOW: u32 = 1 << 9;
pub const QUEUE_ON: u32 = 1 << 16;
pub const QUEUE_BUSY: u32 = 1 << 17;

/// 命令队列 CSR 特有的那两位（故障/页请求队列的 [9] 是溢出，不是这个）。
pub const QUEUE_CMD_TO: u32 = 1 << 9;
pub const QUEUE_CMD_ILL: u32 = 1 << 10;

/// `DDTP.mode`：Off = 设备访存不翻译（隔离态）；1LVL = 一级设备目录表。
pub const DDTP_MODE_OFF: u64 = 0;
pub const DDTP_MODE_BARE: u64 = 1;
pub const DDTP_MODE_1LVL: u64 = 2;
pub const DDTP_BUSY: u64 = 1 << 4;

/// 基址格里的页号字段 [53:10] 与它在格内的移位。
pub const PPN_FIELD: u64 = ((1u64 << 44) - 1) << 10;
pub const PPN_SHIFT: u32 = 10;

/// 翻译请求控制位：go/busy[0]、只读请求[3]、did[63:40]。
pub const TR_GO: u64 = 1 << 0;
pub const TR_NW: u64 = 1 << 3;
pub const TR_DID_SHIFT: u32 = 40;
/// 翻译响应位：fault[0]、ppn[53:10]。
pub const TR_FAULT: u64 = 1 << 0;
pub const TR_PPN_FIELD: u64 = ((1u64 << 44) - 1) << 10;
/// `TR_RESPONSE` 在 FAULT 置起时，原因码落在 ppn 那几格（`riscv_iommu_process_dbg`）。
/// 256 = `DMA_DISABLED`：这台设备此刻不被翻译。
pub const DMA_DISABLED: u64 = 256;

/// 响应寄存器里的原因码（只有 FAULT 置起时才有意义）。
pub fn cause(response: u64) -> u64 {
    (response & TR_PPN_FIELD) >> PPN_SHIFT
}

/// 让硬件按**当前**表翻一次 `iova`（调试口那三格），返响应寄存器原值。
///
/// `device` 是 requester id（DID 那一格）；`write` = 这次请求是写访问
/// （读访问置 `TR_NW`）。调试口走的是与设备访存**同一条**翻译路
/// （`riscv_iommu_process_dbg` → `riscv_iommu_translate`），故它对“这台设备能不能翻”
/// 说的话与设备那一侧同一件事。
pub fn translate(base: usize, device: u32, iova: usize, write: bool) -> u64 {
    write64(base, TR_REQ_IOVA, iova as u64);
    let mut control = TR_GO | ((device as u64) << TR_DID_SHIFT);
    if !write {
        control |= TR_NW;
    }
    write64(base, TR_REQ_CTL, control);
    read64(base, TR_RESPONSE)
}

pub fn read32(base: usize, at: usize) -> u32 {
    // SAFETY: `base` 是一段已授权、已映射的设备寄存器窗口，`at` 是表内的对齐偏移。
    unsafe { core::ptr::read_volatile((base + at) as *const u32) }
}

pub fn read64(base: usize, at: usize) -> u64 {
    // SAFETY: 同 `read32`；`at` 取 8 字节对齐的偏移。
    unsafe { core::ptr::read_volatile((base + at) as *const u64) }
}

pub fn write32(base: usize, at: usize, value: u32) {
    // SAFETY: 同 `read32`。
    unsafe { core::ptr::write_volatile((base + at) as *mut u32, value) }
}

pub fn write64(base: usize, at: usize, value: u64) {
    // SAFETY: 同 `read32`。
    unsafe { core::ptr::write_volatile((base + at) as *mut u64, value) }
}

/// 基址格里的页号：物理页起址右移一页。
pub fn page_number(physical: usize) -> u64 {
    (physical as u64 >> 12) & (PPN_FIELD >> PPN_SHIFT)
}

/// 有界轮询：写完之后等硬件把 `BUSY` 落下去。
///
/// 与 Linux `iommu/riscv/iommu.c` 同一条口径（`RISCV_IOMMU_DDTP_TIMEOUT`）：
/// 写下去还要轮询到 busy 落，写后单次读不算走完协议。
const POLL_LIMIT: usize = 10_000_000;

/// 换 `DDTP`：`mode` 与（`xLVL` 时的）根表页号，按规范那一手走完——
/// 写 → 有界轮询到 busy 落 → 回读校验 mode 真的是要的那一档。
///
/// 返最后那一刻的真实读数（失败现场也要有数）。
pub fn ddtp_set(base: usize, mode: u64, root: Option<usize>) -> Result<u64, (&'static str, u64)> {
    if mode > DDTP_MODE_BARE && root.is_none() {
        return Err(("iommu: ddtp root", 0));
    }
    let mut value = mode;
    if let Some(root) = root {
        value |= page_number(root) << PPN_SHIFT;
    }
    write64(base, DDTP, value);
    let mut seen = read64(base, DDTP);
    let mut left = POLL_LIMIT;
    while seen & DDTP_BUSY != 0 && left > 0 {
        seen = read64(base, DDTP);
        left -= 1;
    }
    if seen & DDTP_BUSY != 0 || seen & 0xF != mode {
        return Err(("iommu: ddtp handshake", seen));
    }
    Ok(seen)
}
