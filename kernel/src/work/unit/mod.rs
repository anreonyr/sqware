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
// 装配（内核 Space 构建 + MMU 初始化）仍在 memory::manager；本模块负责把构建好的
// 内核空间封包进 KERNEL_TEAM（team::init_kernel），并承载 Space 相关的自测。

pub mod elftable;
pub(crate) mod loader;
pub(crate) mod parser;
pub mod space;
pub(crate) mod task;
pub(crate) mod team;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::allocator::block::live_pages;
use crate::memory::allocator::frame::{allocator, FRAME_ALLOCATOR};
use crate::memory::manager::entry::PteFlags;
use crate::memory::PAGE_SIZE;

use space::{MapKind, SpaceBuilder};

/// PT 回收自测（debug）：map/unmap 循环验证中间表回收——无孤儿表、无 double-free。
///
/// 在 spawn 用户任务之前运行（分配器与 KERNEL_TEAM 均已就绪），由 `boot::init`
/// 经 `crate::work::unit::pt_reclaim_selftest()` 调用。每轮：
/// map 4 MiB（4 KiB 页，根表槽 1）→ 表数 +3（1×L1 + 2×L0）；unmap → 回落；
/// 32 轮后「在途帧 − 堆支撑页」回到轮前（块堆缓存页不误报，口径同 check_baseline）。
#[cfg(debug_assertions)]
pub fn pt_reclaim_selftest() {
    const BASE: usize = 0x4000_0000; // 根表槽 1：堆窗口之后、栈窗口之前的空地
    const SIZE: usize = 4 * 1024 * 1024; // 4 MiB → 1×L1 + 2×L0
    const ROUNDS: usize = 32;

    let space = SpaceBuilder::user().build().expect("selftest: build space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
    let base_count = space.table_count();
    let held_before = FRAME_ALLOCATOR
        .outstanding()
        .saturating_sub(live_pages());

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

    let held_after = FRAME_ALLOCATOR.outstanding().saturating_sub(live_pages());
    assert_eq!(
        held_before, held_after,
        "selftest: net frames leaked: {held_before} → {held_after}"
    );
    drop(space);
    crate::putln!("pt-reclaim selftest: ok ({ROUNDS} rounds, tables {base_count} → +3 → {base_count})");
}
