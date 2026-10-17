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

/// 一次有界握手的期限：建立时定下，**跨帧保留**。
///
/// 交出去之后可以反复问（[`Handshake::expired`]），期限不再变——这正是
/// 「一帧只读一次、没等到就留到下一帧」与「重复调用不重置时限」共同要的那一格。
#[derive(Clone, Copy)]
pub struct Handshake {
    until: u64,
}

impl Handshake {
    /// 从这一刻起给这次握手 `limit` 纳秒。
    pub fn new(limit: u64) -> Self {
        Self {
            until: env::chrono::clock().saturating_add(limit),
        }
    }

    /// 期限过了没有。
    pub fn expired(&self) -> bool {
        env::chrono::clock() >= self.until
    }

    /// 同步那一档：读到 `done` 成立或到期限为止，返最后那一刻的读数。
    /// 跨帧那一档不用它——那一路每帧只读一次，期限由 [`Handshake::expired`] 判。
    pub fn wait<T: Copy>(&self, mut read: impl FnMut() -> T, done: impl Fn(T) -> bool) -> T {
        let mut seen = read();
        while !done(seen) && !self.expired() {
            seen = read();
        }
        seen
    }
}

/// `DDTP` 写下去之后等 `busy` 落的期限。
///
/// 与 Linux `iommu/riscv/iommu.c` 同一条口径（`RISCV_IOMMU_DDTP_TIMEOUT`）：
/// 写后单次读不算走完协议。原来是十的七次方次**自旋计数**，现在是一条期限：
/// 跨帧那一档靠 [`Handshake::expired`] 到点，同步那一档靠 [`Handshake::wait`] 到点。
const DDTP_NS: u64 = 2_000_000_000;

/// 换 `DDTP` 的第一步：校验参数、写下 `mode`（`xLVL` 时带上根表页号），并起一次握手。
///
/// 与 [`ddtp_poll`] 成对：写只做一次，回读归单次那一格。
pub fn ddtp_start(
    base: usize,
    mode: u64,
    root: Option<usize>,
) -> Result<Handshake, (&'static str, u64)> {
    if mode > DDTP_MODE_BARE && root.is_none() {
        return Err(("iommu: ddtp root", 0));
    }
    let mut value = mode;
    if let Some(root) = root {
        value |= page_number(root) << PPN_SHIFT;
    }
    write64(base, DDTP, value);
    Ok(Handshake::new(DDTP_NS))
}

/// 换 `DDTP` 的下一步：**单次读**。`busy` 落了且 mode 真的是要的那一档 → `Ok(seen)`。
///
/// `busy` 还挂着 = 这一步还没走完，返 `("iommu: ddtp busy", seen)`——调用方按 [`Handshake`]
/// 判「再等一帧」还是「期限到了」。`busy` 落了但 mode 不对 = 不认识的现场，
/// 返 `("iommu: ddtp handshake", seen)`。
pub fn ddtp_poll(base: usize, mode: u64) -> Result<u64, (&'static str, u64)> {
    let seen = read64(base, DDTP);
    if seen & DDTP_BUSY != 0 {
        return Err(("iommu: ddtp busy", seen));
    }
    if seen & 0xF != mode {
        return Err(("iommu: ddtp handshake", seen));
    }
    Ok(seen)
}

/// 换 `DDTP` 的同步那一档（探针/引导）：写 → 轮询到 busy 落 → 回读校验 mode。
///
/// 返最后那一刻的真实读数（失败现场也要有数）。
pub fn ddtp_set(base: usize, mode: u64, root: Option<usize>) -> Result<u64, (&'static str, u64)> {
    let handshake = ddtp_start(base, mode, root)?;
    loop {
        match ddtp_poll(base, mode) {
            Ok(seen) => return Ok(seen),
            Err((_, seen)) if seen & DDTP_BUSY != 0 && !handshake.expired() => continue,
            Err((_, seen)) => return Err(("iommu: ddtp handshake", seen)),
        }
    }
}
