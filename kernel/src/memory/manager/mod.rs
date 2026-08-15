// 虚拟内存管理 — Sv39 页表 + 地址空间 + 缺页 + ASID
//
// 子模块：
//   addr      — VirtAddr / PhysAddr
//   entry     — Sv39 PTE + PteFlags
//   fault     — 缺页处理
//   space     — Space/SpaceKind、Map/Window/Durable 簿记模型、内核地址空间初始化
//   table     — PageTable、页表遍历/映射（pub(crate)）
//   asid      — ASID 分配器
//
// satp/TLB 切换函数（satp_token / flush_asid）与 MMU 初始化（init）归本模块，
// 描述「如何管理虚拟地址空间」，与物理内存分配（crate::memory::allocator）解耦。
// 地址空间布局常量（USER_HEAP_* / TASK_STACK_*）收敛进 space 模块，
// 由通用位图分配器实例（memory::allocator::bitmap）消费；ASID 空间亦为其实例。

pub mod addr;
pub mod asid;
pub mod entry;
pub mod fault;
pub mod space;
pub mod table;

use crate::{
    machine,
    memory::{
        PAGE_SIZE,
        allocator::frame::allocator,
        manager::{
            addr::{PhysAddr, VirtAddr},
            entry::PteFlags,
            space::{
                KERNEL_BASE, KERNEL_SPACE, KERNEL_TRAP_CONTEXT, MapKind, SpaceBuilder, TRAMPOLINE,
                TRAP_CONTEXT, USER_STACK_BASE,
            },
        },
    },
};

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use erra::ResultExt;

use riscv::register::satp;
/// 页表操作错误 — `Space` pub 方法返回的错误类型。
///
/// 经 `pub use` 从 `pub(crate) mod table` 导出，使 pub API 签名中的类型
/// 可通过 `crate::memory::manager::MapError` 命名。bin crate 无外部消费者，
/// re-export 为「pub 签名类型可命名性」预留，故 allow(unused_imports)。
#[allow(unused_imports)]
pub use table::MapError;

/// 页表/MMU 操作结果 — `erra::Error<MapError>` 附加调用点上下文。
pub type MapResult<T> = erra::Result<T, MapError>;

/// 刷新指定 ASID 的 TLB 条目（非全局）。
///
/// 本地 `sfence.vma zero, asid`（rs2 用通用寄存器传值：asid=0 时全刷
/// 含全局条目，asid≠0 时只刷新该 ASID）。页表修改（map/unmap/protect）后
/// 按空间 ASID 调用。
///
/// **无需远程刷（RFNC）**：每次 satp 切换（`__alltraps`/`__restore`）都已全刷
/// 本地 TLB（rs1=rs2=x0），跨核不会残留陈旧条目——远程核只在切换 satp 到自己
/// 空间时接触其页表，而切换本身即全刷；内核空间 post-boot 无 map/unmap。休眠
/// 核（WFI）醒来后同样经 satp 切换全刷，不依赖远程 fence。
///
/// # Safety
///
/// 调用者需确保刷新后页表仍然有效。
#[inline(always)]
pub unsafe fn flush_asid(asid: usize) {
    unsafe {
        core::arch::asm!("sfence.vma zero, {}", in(reg) asid);
    }
}

/// 初始化 MMU：创建内核地址空间，identity-map DRAM 和 MMIO，启用 Sv39 分页。
///
/// 必须在 `memory::allocator::init()` 之后、在驱动程序 MMIO 访问之前调用。
///
/// # Safety
///
/// 写入 `satp` 后会立即启用分页。调用者需确保此时所有存活的指针
/// （栈、代码、数据段）都已 identity-mapped。
///
/// # Errors
///
/// - [`MapError::DramOverlap`] — DRAM 末端越过用户栈窗口（内存配置非法）。
/// - [`MapError::OutOfMemory`] — 物理帧不足以分配根/中间页表或内核 trap-context 帧。
/// - [`MapError::NotAligned`] / [`MapError::AlreadyMapped`] — 映射参数非法。
pub fn init() -> MapResult<()> {
    (|| -> Result<(), MapError> {
        unsafe {
            let m = machine::get();

            // 任务栈窗口 TASK_STACK_BASE=0xC0000000：恒等映射的 DRAM 必须落在其下方，
            // 否则任务栈窗口覆盖真实内存而非专用窗口（DRAM 起点 0x80000000 → size < 1 GiB）。
            if VirtAddr::from_raw(m.dram.base + m.dram.size) > USER_STACK_BASE {
                return Err(MapError::DramOverlap);
            }

            // 1. 创建内核地址空间
            let kernel_space = SpaceBuilder::kernel().build()?;

            // 2. Identity-map 整个 DRAM —— 内核镜像（_free_base 以下）+ free 区 +
            //    内核栈保留区（DRAM 顶部）都在 DRAM 内。只 map free 会在启用分页后
            //    让内核栈/内核镜像变成未映射，下一次栈访问或取指即缺页。
            let ram_flags = PteFlags::V
                | PteFlags::R
                | PteFlags::W
                | PteFlags::X
                | PteFlags::A
                | PteFlags::D
                | PteFlags::G;

            kernel_space.map(
                VirtAddr::from_raw(m.dram.base),
                PhysAddr::from_raw(m.dram.base),
                m.dram.size,
                ram_flags,
                MapKind::Reserved, // 借用映射：帧归机器/内核；user 半区触碰 → 预留诊断
                Vec::new(),
            )?;

            // 3. 内核高半区映射（同样覆盖整个 DRAM，为 S-mode 切换做准备）
            kernel_space.map(
                KERNEL_BASE + m.dram.base,
                PhysAddr::from_raw(m.dram.base),
                m.dram.size,
                ram_flags,
                MapKind::Reserved,
                Vec::new(),
            )?;

            // 4. 映射 trap trampoline 页（内核自有帧）：所有空间以 TRAMPOLINE VA
            //    映射同一物理页，`stvec` 指向它。G 位：内容不可变，不被 ASID 局部
            //    sfence 刷掉也安全。
            let tramp_flags =
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::A | PteFlags::D | PteFlags::G;
            kernel_space.map(
                TRAMPOLINE,
                crate::runtime::trampoline::trampoline_pa(),
                PAGE_SIZE,
                tramp_flags,
                MapKind::Reserved,
                Vec::new(),
            )?;

            // 5. 内核 trap-context 帧：映射于 TRAP_CONTEXT（内核自身 trap 用；
            //    元数据字段由 trap::init 写入），帧入常数 Map，PA 存入 KERNEL_TRAP_CONTEXT。
            let ktc = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                .map_err(|_| MapError::OutOfMemory)?;
            let ktc_pa = PhysAddr::from_raw(ktc.as_ptr() as usize);
            kernel_space.map(
                TRAP_CONTEXT,
                ktc_pa,
                PAGE_SIZE,
                PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
                MapKind::Anonymous,
                vec![ktc],
            )?;
            KERNEL_TRAP_CONTEXT.store(ktc_pa, core::sync::atomic::Ordering::Relaxed);

            // 6. 启用 Sv39 分页
            satp::set(satp::Mode::Sv39, 0, kernel_space.root());

            // 7. 刷新 TLB
            flush_asid(0);

            // 8. 保存内核地址空间
            KERNEL_SPACE.lock().replace(kernel_space);

            Ok(())
        }
    })()
    .annotate("initializing memory manager")
}
