// 健康检查 · pt_reclaim — PT 回收自测：map/unmap 循环验证中间表回收，
// 无孤儿表、无 double-free。
//
// 每轮：map 4 MiB（4 KiB 页，根表槽 1）→ 表数 +3（1×L1 + 2×L0）；unmap → 回落；
// 32 轮后「在途帧 − 堆支撑页」回到轮前。断言用 `expect!`：失败统一报告 + fail-fast。

#![cfg(debug_assertions)]

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::SpaceBuilder;

/// PT 回收自测（audit-only）：map/unmap 循环验证中间表当场归还。
pub fn pagetable() {
    // 表数期望随模式层级（4 MiB = 2×L0 + 每层一个中间表 = 共 levels 张表）。
    let levels = crate::memory::manager::mode::geometry(crate::memory::manager::mode::mode()).levels
        as usize;
    const BASE: usize = 0x4000_0000; // 根表槽 1：堆窗口之后、栈窗口之前的空地
    const SIZE: usize = 4 * 1024 * 1024; // 4 MiB → 2×L0 + (levels−2) 中间表
    const ROUNDS: usize = 32;

    let space = SpaceBuilder::user()
        .build()
        .expect("[health] pagetable: build space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
    let base_count = space.table_count();
    // 全动态下块池页已计入 frame outstanding，剔除块池持页——期间 spare 迟滞
    // 保留的页会净增 outstanding，不误报。
    let held_before = crate::memory::allocator::frame::outstanding()
        - crate::memory::allocator::block::held_pages();

    for round in 0..ROUNDS {
        // map：分配数据帧 + 中间表
        let mut frames: Vec<crate::memory::manager::table::Frame> = Vec::new();
        for _ in 0..(SIZE / PAGE_SIZE) {
            frames.push(unsafe {
                Box::try_new_zeroed_in(crate::memory::allocator::frame::allocator())
                    .expect("[health] pagetable: data frame")
                    .assume_init()
            });
        }
        let pa = PhysAddr::from_raw(frames[0].as_ptr() as usize);
        space
            .map(
                VirtAddr::from_raw(BASE),
                pa,
                SIZE,
                flags,
                frames,
            )
            .expect("[health] pagetable: map");
        crate::expect!(
            space.table_count() == base_count + levels,
            "round {round}: tables after map (got {} want {})",
            space.table_count(),
            base_count + levels
        );
        crate::expect!(
            space.translate(VirtAddr::from_raw(BASE)).is_some(),
            "round {round}: map hit"
        );

        // unmap：回收中间表 + 数据帧（树自底向上判空摘除；double-free 由分配器检测）
        space.unmap(VirtAddr::from_raw(BASE), SIZE);
        crate::expect!(
            space.table_count() == base_count,
            "round {round}: tables after unmap (got {} want {})",
            space.table_count(),
            base_count
        );
        crate::expect!(
            space.translate(VirtAddr::from_raw(BASE)).is_none(),
            "round {round}: unmap hit"
        );
    }

    let held_after = crate::memory::allocator::frame::outstanding()
        - crate::memory::allocator::block::held_pages();
    crate::expect!(
        held_before == held_after,
        "net frames leaked: {held_before} → {held_after}"
    );
    drop(space);
    super::report_ok(
        "pagetable",
        format_args!(
            "{ROUNDS} rounds, levels {levels}, tables {base_count} → +{levels} → {base_count}"
        ),
    );
}
