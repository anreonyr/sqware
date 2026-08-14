// memory 内部架构层 — 自包含的 RISC-V 原语
//
// 原依赖内核 hal（cpu::hart_id / csr::satp），现内化于此——复制 memory 到
// 其他 RISC-V 内核项目无需携带内核 hal。satp 构造为纯位运算；hart_id 为
// 单 hart stub。

/// 当前 hart id — 读 mhartid CSR。
///
/// # Safety
///
/// 处于 S-mode；读 mhartid 无副作用。单 hart 启动下 mhartid = 0，行为与旧
/// stub 一致；多 hart 启动协议就绪后 block 分配器的 per-hart 槽自动生效。
#[inline(always)]
pub(crate) unsafe fn hart_id() -> usize {
    riscv::register::mhartid::read()
}

/// Sv39 satp token 构造（纯位运算，不碰 CSR）。
pub(crate) mod satp {
    /// MODE: Sv39（三级页表）
    pub const MODE_SV39: usize = 8;

    const MODE_SHIFT: usize = 60;

    /// 构造 satp 值: MODE | (ASID << 44) | PPN
    #[inline(always)]
    pub const fn make(mode: usize, asid: usize, ppn: usize) -> usize {
        (mode << MODE_SHIFT) | ((asid & 0xFFFF) << 44) | (ppn & 0x000F_FFFF_FFFF)
    }
}
