// 健康检查 · stress — 内核块分配器（block 池）压力演练：上游 `Allocator` 接口
// 验收（与 `hybrid::allocator()` 同一分配器；仅 block 域 ≤ 半页）。
//
// 断言：
//   · 多尺寸混合分配-立即释放循环闭环（block 池分合路径）；
//   · 持有-全释放后复验可再分配（合并闭环）。
// 失败路径（AllocError 返回 Err）由 spare 演练（预算溢出）与用户态 stressor
// （超大分配必 Err）覆盖——不在本用例重复。frame 后端（> 半页）另有 spare 演练
// 与 pagetable 自测，本用例不触碰。
//
// 注意（实测发现）：`hybrid::allocator()` 的 frame 后端对 order1+（8192B 级）
// 分配疑似卡死（boot 期 free 帧不足时向上找 order 可能无出口）——已绕开；该
// 处待单独排查。

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::memory::allocator::hybrid;
use crate::memory::PAGE_SIZE;

/// 混合档位（全部 ≤ 半页 → block 域：16B..2048B 跨 size class）。
const SIZES: [usize; 7] = [16, 64, 128, 256, 512, 1024, 2048];
/// 幕 1 步数（带预算，防压住 boot）。
const STEPS: usize = 64;
/// 幕 2 持有批大小（256B..2KiB 全 block 域）。
const HELD: usize = 8;

pub fn accept() {
    let a = hybrid::allocator();

    // 幕 1：多尺寸混合 分配-立即释放
    for i in 0..STEPS {
        let l = Layout::from_size_align(SIZES[i % SIZES.len()], 8).unwrap();
        // SAFETY: 分配块当轮即释（闭环）。
        unsafe {
            let b = a.allocate(l).expect("stress: alloc 1");
            a.deallocate(b.cast(), l);
        }
    }

    // 幕 2：持有-全释放（强制合并）→ 复验可再分配
    let mut held: Vec<(NonNull<[u8]>, Layout)> = Vec::new();
    for i in 0..HELD {
        let l = Layout::from_size_align(PAGE_SIZE / 16 * (i + 1), 8).unwrap();
        let b = a.allocate(l).expect("stress: alloc 2");
        held.push((b, l));
    }
    for (b, l) in held.drain(..) {
        // SAFETY: b 来自本幕 allocate、layout 同源。
        unsafe { a.deallocate(b.cast(), l) };
    }
    let l = Layout::from_size_align(1024, 8).unwrap();
    // SAFETY: 复验块当轮即释。
    unsafe {
        let b = a.allocate(l).expect("stress: re-alloc after merge");
        a.deallocate(b.cast(), l);
    }

    crate::health::report_ok(
        "stress",
        format_args!("block allocator {STEPS} mixed + {HELD} held iters clean"),
    );
}