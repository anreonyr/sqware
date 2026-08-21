// 任务执行单元（unit）— 地址空间 + 团队 + 线程 + 装载，物化为一个子模块
//
// 一个 Team 持有唯一 Space（共享地址空间），多个 Task 共享之；每个 Task 持有
// 自己的 trap 帧（Frame 窗口分配，任意 VA——alltraps/restore 经帧内 self_va
// 定位）。由 S-timer 抢占 + envcall 驱动切换。切换完全走 trap 链路——
// trap_handler 返回下一任务帧 → restore 切 satp + sret，无独立切换汇编。
//
// 子模块：
//   space     — 地址空间（Space/SpaceBuilder、Map/Window/Durable 簿记模型、内核布局）
//   team      — 团队容器（Team/TeamBuilder/kernel 单例；内核空间唯一归属 KERNEL_TEAM）
//   task      — 线程单元（Task/TaskBuilder）
//   loader    — 程序装载（ELF → Space durable）
//   parser    — ELF 解析（含符号表抽取）
//   elftable  — 符号表
//
// `init` 是本子系统的唯一装配入口：构建内核地址空间（identity-map DRAM /
// 高半区 / rodata / TRAMPOLINE / per-hart 内核帧）、启用 Sv39 分页，并把它
// 封包进 KERNEL_TEAM。原本的 memory::manager::init 已并入此处（satp/DRAM 装配
// 随之迁来），memory::manager 只留原语/错误接缝。

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
use crate::memory::allocator::frame::{allocator, outstanding};
use crate::memory::manager::{
    MapError,
    addr::{PhysAddr, VirtAddr},
    entry::PteFlags,
    flush_asid,
};

use space::{
    KERNEL_BASE, KERNEL_FRAME_BASE, MapKind, SpaceBuilder, TRAMPOLINE, USER_STACK_BASE,
    init_kernel_frames, kernel_frames, trampoline_pa,
};

// 链接脚本 `.rodata` 起始（镜像尾部只读段）——内核映射时将其置为只读，
// 兼作主栈下方的写保护 guard（栈下溢踩 .rodata 即写保护缺页）。
unsafe extern "C" {
    static _rodata_start: u8;
}

/// 页表/MMU 操作结果 — `erra::Error<MapError>` 附加调用点上下文。
pub type MapResult<T> = erra::Result<T, MapError>;

/// 初始化 MMU：创建内核地址空间，identity-map DRAM 和 MMIO，启用 Sv39 分页，
/// 并把内核空间封包进 KERNEL_TEAM（唯一出生点）。
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
            let m = machine::info();

            // 任务栈窗口 TASK_STACK_BASE=0xC0000000：恒等映射的 DRAM 必须落在其下方，
            // 否则任务栈窗口覆盖真实内存而非专用窗口（DRAM 起点 0x80000000 → size < 1 GiB）。
            if VirtAddr::from_raw(m.dram.base + m.dram.size) > USER_STACK_BASE {
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

            // 3. 内核高半区映射（同样覆盖整个 DRAM，为 S-mode 切换做准备）
            kernel_space.map(
                KERNEL_BASE + m.dram.base,
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
            kernel_space.protect(KERNEL_BASE + rodata_start, rodata_size, ro_flags)?;

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

            // 5. per-hart 内核 trap-context 帧：KERNEL_FRAME_BASE 起 N 页（hart h 帧 =
            //    BASE + h·PAGE；元数据由 trap::init 逐帧写入）。帧 PA 表按实际核数
            //    动态分配（不再编译期预留 MAX 槽的静态数组）；PA 存 frames[h]——
            //    __strap 按 TP 索引的是帧区 VA（KERNEL_FRAME_BASE），不经此表。
            let n = machine::hart_count();
            init_kernel_frames(n);
            let frames = kernel_frames();
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

            // 6. 启用 Sv39 分页
            satp::set(satp::Mode::Sv39, 0, kernel_space.root());

            // 7. 刷新 TLB
            flush_asid(0);

            // 8. 把内核地址空间封包进内核团队（KERNEL_TEAM 唯一持有；KERNEL_SPACE
            //    全局已消除，团队插入此处即内核空间的唯一出生点）
            team::init_kernel(Arc::new(kernel_space));

            Ok(())
        }
    })()
    .annotate("initializing unit (kernel space + team)")
}

/// PT 回收自测（debug）：map/unmap 循环验证中间表回收——无孤儿表、无 double-free。
///
/// 在 spawn 用户任务之前运行（分配器与 KERNEL_TEAM 均已就绪），由 `boot::init`
/// 经 `crate::work::unit::pagetable_reclaim()` 调用。每轮：
/// map 4 MiB（4 KiB 页，根表槽 1）→ 表数 +3（1×L1 + 2×L0）；unmap → 回落；
/// 32 轮后「在途帧 − 堆支撑页」回到轮前（块堆缓存页不误报，口径同 check_baseline）。
#[cfg(debug_assertions)]
pub fn pagetable_reclaim() {
    const BASE: usize = 0x4000_0000; // 根表槽 1：堆窗口之后、栈窗口之前的空地
    const SIZE: usize = 4 * 1024 * 1024; // 4 MiB → 1×L1 + 2×L0
    const ROUNDS: usize = 32;

    let space = SpaceBuilder::user().build().expect("selftest: build space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
    let base_count = space.table_count();
    let held_before = outstanding();

    for round in 0..ROUNDS {
        // map：分配数据帧 + 中间表
        let mut frames = Vec::new();
        for _ in 0..(SIZE / PAGE_SIZE) {
            frames.push(
                Box::try_new_in([0u8; PAGE_SIZE], allocator()).expect("selftest: data frame"),
            );
        }
        let pa = crate::memory::manager::addr::PhysAddr::from_raw(frames[0].as_ptr() as usize);
        space
            .map(
                crate::memory::manager::addr::VirtAddr::from_raw(BASE),
                pa,
                SIZE,
                flags,
                MapKind::Anonymous,
                frames,
            )
            .expect("selftest: map");
        assert_eq!(
            space.table_count(),
            base_count + 3,
            "selftest round {round}: tables after map"
        );
        assert!(
            space
                .translate(crate::memory::manager::addr::VirtAddr::from_raw(BASE))
                .is_some(),
            "selftest round {round}: map hit"
        );

        // unmap：回收中间表 + 数据帧（树自底向上判空摘除；double-free 由分配器检测）
        space.unmap(crate::memory::manager::addr::VirtAddr::from_raw(BASE), SIZE);
        assert_eq!(
            space.table_count(),
            base_count,
            "selftest round {round}: tables after unmap"
        );
        assert!(
            space
                .translate(crate::memory::manager::addr::VirtAddr::from_raw(BASE))
                .is_none(),
            "selftest round {round}: unmap hit"
        );
    }

    let held_after = outstanding();
    assert_eq!(
        held_before, held_after,
        "selftest: net frames leaked: {held_before} → {held_after}"
    );
    drop(space);
    crate::putln!(
        "pagetable reclaim test: ok ({ROUNDS} rounds, tables {base_count} → +3 → {base_count})"
    );
}
