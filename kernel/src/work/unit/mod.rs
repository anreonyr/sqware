// 任务执行单元（unit）— 地址空间 + 团队 + 线程 + 装载。
//
// 一个 Team 持有唯一 Space（共享地址空间），多个 Task 共享之；每个 Task 持有
// 自己的 trap 帧（Frame 窗口分配）。
//
//   space     — 地址空间（Space/SpaceBuilder、Map/Window/Durable 簿记模型、内核布局）
//   team      — 团队容器（Team/TeamBuilder/kernel 单例）
//   task      — 线程单元（Task/TaskBuilder）
//   loader    — 程序装载（ELF → Space durable）
//   parser    — ELF 解析（含符号表抽取）
//   elftable  — 符号表

pub mod elftable;
pub(crate) mod loader;
pub(crate) mod parser;
pub mod space;
pub(crate) mod task;
pub(crate) mod team;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use erra::ResultExt;

use riscv::register::satp;

use crate::machine::{self, kernel_edge};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::{
    MapError,
    addr::{PhysAddr, VirtAddr},
    entry::PteFlags,
    flush_asid,
    mode,
};

use space::{
    KERNEL_FRAME_BASE, MapKind, SpaceBuilder, TRAMPOLINE, init_kernel_frames, trampoline_pa,
};

// 链接脚本 `.rodata` 起始（镜像尾部只读段）——内核映射时将其置为只读，
// 兼作主栈下方的写保护 guard（栈下溢踩 .rodata 即写保护缺页）。
unsafe extern "C" {
    static _rodata_start: u8;
}

/// 页表/MMU 操作结果 — `erra::Error<MapError>` 附加调用点上下文。
pub type MapResult<T> = erra::Result<T, MapError>;

/// 初始化 MMU：**先探测 satp 模式**（P1：最小恒等临时根，候选 57→48→39），
/// 再创建内核地址空间，identity-map DRAM 和 MMIO，启用探测所得分页模式，
/// 并把内核空间封包进 KERNEL_TEAM。
///
/// 必须在分配器初始化之后调用（探测临时根表取帧）。
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
/// - satp 模式探测失败（全不支持/帧耗尽）→ panic（boot 级致命）。
pub fn init() -> MapResult<()> {
    (|| -> Result<(), MapError> {
        unsafe {
            let m = machine::info();

            // 0. 探测 satp 模式并部署（此后 mode()/几何按它派生）。失败 = 无
            //    S 态分页或帧耗尽——boot 级致命。
            mode::detect().unwrap_or_else(|e| panic!("satp mode detect failed: {e:?}"));

            // 任务栈窗口顶锚于用户半区顶（mode::upper 起）：恒等映射的 DRAM
            // 必须落在其下方，否则任务栈窗口覆盖真实内存而非专用窗口。
            if VirtAddr::from_raw(m.dram.base + m.dram.size) > mode::upper() {
                return Err(MapError::DramOverlap);
            }

            // 1. 创建内核地址空间
            let kernel_space = SpaceBuilder::kernel().build()?;

            // 2. Identity-map 整个 DRAM —— 内核镜像（含镜像内主栈区，位于
            //    `_kernel_edge` 之上）都在 DRAM 内。只 map free 会在启用分页后
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

            // 3. 内核高半区映射（同样覆盖整个 DRAM，为 S-mode 切换做准备；
            //    高半区起点 = 探测模式的 lower()，随模式）
            kernel_space.map(
                mode::lower() + m.dram.base,
                PhysAddr::from_raw(m.dram.base),
                m.dram.size,
                ram_flags,
                MapKind::Reserved,
                Vec::new(),
            )?;

            // 3.5 内核 .rodata 段只读化：镜像尾部 .rodata 经恒等与高半区两处都已
            //    RWX 映射，此处用 protect 降为只读（去 W）。作用有二：
            //      a) 主栈位于 _kernel_edge 之上、向下生长，越界第一脚即踩 .rodata
            //         → 写保护缺页（天然主栈 guard，省 unmap/预留帧）；
            //      b) 内核只读数据获得 RO 防护（BUG 改写 .rodata 立即缺页暴露）。
            //    protect 只改已映射叶子 PTE，不影响中间表与 free 区；两处都要降。
            let rodata_start = (&raw const _rodata_start).addr();
            let rodata_size = kernel_edge() - rodata_start;
            let ro_flags = PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D | PteFlags::G;
            kernel_space.protect(VirtAddr::from_raw(rodata_start), rodata_size, ro_flags)?;
            kernel_space.protect(mode::lower() + rodata_start, rodata_size, ro_flags)?;

            // 4. 映射 trap trampoline 页（内核自有帧）：所有空间以 TRAMPOLINE VA
            //    映射同一物理页，`stvec` 指向它。G 位：内容不可变，不被 ASID 局部
            //    sfence 刷掉也安全。
            let tramp_flags =
                PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::A | PteFlags::D | PteFlags::G;
            kernel_space.map(
                TRAMPOLINE,
                trampoline_pa(),
                PAGE_SIZE,
                tramp_flags,
                MapKind::Reserved,
                Vec::new(),
            )?;

            // 5. per-hart 内核 trap-context 帧：KERNEL_FRAME_BASE 起 N 页。
            let n = machine::hart_count();
            let frames = init_kernel_frames(n);
            for (h, slot) in frames.iter().enumerate() {
                let page = Box::try_new_in([0u8; PAGE_SIZE], allocator())
                    .map_err(|_| MapError::OutOfMemory)?;
                let pa = PhysAddr::from_raw(page.as_ptr() as usize);
                kernel_space.map(
                    KERNEL_FRAME_BASE + h * PAGE_SIZE,
                    pa,
                    PAGE_SIZE,
                    PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
                    MapKind::Anonymous,
                    vec![page],
                )?;
                slot.store(pa, core::sync::atomic::Ordering::Relaxed);
            }

            // 6. 启用探测所得模式的分页（satp MODE 字段随模式）
            satp::set(mode::mode(), 0, kernel_space.root());

            // 7. 刷新 TLB + 运行期布局校验（debug：违例 fail-fast）
            flush_asid(0);
            #[cfg(debug_assertions)]
            space::validate();

            // 8. 内核空间封包进内核团队。
            team::init_kernel(Arc::new(kernel_space));

            Ok(())
        }
    })()
    .annotate("initializing unit (kernel space + team)")
}

