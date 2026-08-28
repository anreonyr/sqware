/*
健康检查 · stress — 内核分配器压力演练：上游 `Allocator` 接口验收
（与 `hybrid::allocator()` 同一分配器）。

断言：
  · 多尺寸混合分配-立即释放循环闭环（block 池分合路径，≤ 半页）；
  · 持有-全释放后复验可再分配（合并闭环）；
  · frame 后端档位闭环（> 半页：order0..3 分配-立即释放）；
  · frame 持有-全释放（跨 order 分裂/合并交错）；
  · frame 耗尽-反还：order1 档不断分配直至帧池耗尽（AllocError），全量归还
    后复验可再分配——「boot 期 free 帧不足时向上找 order 无出口」的直接
    暴露点：耗尽后 split_block 必须返回 None 而非挂死。

实测结论（QEMU 双核、debug 构建全绿）：当初「frame 后端 order1+（8192B 级）
分配疑似卡死」不可复现——order0..3 多档闭环、持有交错、耗尽榨干-全归复验
均正常（耗尽可用 4122 块 order1 ≈ 32 MiB 后正确 Err）。该怀疑源自早期
探针观察（health 后断流疑为 timeout 内未完成，非挂死），现已以本用例固化
为长期回归。
*/

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc::vec::Vec;

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::hybrid;

/// 幕 1 block 档位（全部 ≤ 半页：16B..2048B 跨 size class）。
const SIZES: [usize; 7] = [16, 64, 128, 256, 512, 1024, 2048];
/// 幕 1 步数（带预算，防压住 boot）。
const STEPS: usize = 64;
/// 幕 2 持有批大小（256B..2KiB 全 block 域）。
const HELD: usize = 8;
/// 幕 3 frame 档位（> 半页：4096=order0 .. 32768=order3）。
const FRAME_SIZES: [usize; 4] = [4096, 8192, 16384, 32768];
/// 幕 3 步数（每档循环分配-释放闭环）。
const FRAME_STEPS: usize = 64;
/// 幕 4 持有批大小（跨 order 分裂/合并交错；block 幕 2 的 frame 对偶）。
const FRAME_HELD: usize = 8;

pub fn accept() {
    let a = hybrid::allocator();

    // 幕 1：block 域 多尺寸混合 分配-立即释放
    for i in 0..STEPS {
        let l = Layout::from_size_align(SIZES[i % SIZES.len()], 8).unwrap();
        // SAFETY: 分配块当轮即释（闭环）。
        unsafe {
            let b = a.allocate(l).expect("stress: alloc 1");
            a.deallocate(b.cast(), l);
        }
    }

    // 幕 2：block 持有-全释放（强制合并）→ 复验可再分配
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

    // 幕 3：frame 后端多档闭环（> 半页 → 内存直取，不走 block 池）。
    for i in 0..FRAME_STEPS {
        let size = FRAME_SIZES[i % FRAME_SIZES.len()];
        let l = Layout::from_size_align(size, PAGE_SIZE).unwrap();
        // SAFETY: 块当轮即释（闭环）。
        unsafe {
            let b = a.allocate(l).expect("stress: frame alloc");
            a.deallocate(b.cast(), l);
        }
    }

    // 幕 4：frame 持有-全释放（跨 order 分裂/合并交错）。
    let mut fheld: Vec<(NonNull<[u8]>, Layout)> = Vec::new();
    for i in 0..FRAME_HELD {
        let l = Layout::from_size_align(PAGE_SIZE * (1 << (i % 4)), PAGE_SIZE).unwrap();
        let b = a.allocate(l).expect("stress: frame hold alloc");
        fheld.push((b, l));
    }
    for (b, l) in fheld.drain(..) {
        // SAFETY: b 来自本幕 allocate、layout 同源。
        unsafe { a.deallocate(b.cast(), l) };
    }

    // 幕 5：frame 耗尽-反还——order1 档不断分配直至帧池耗尽（AllocError），
    // 再全量归还并复验。耗尽后 split_block 必须返回 None（向上找 order 有界：
    // `k < freelist.len()` 单调递增），而非挂死。
    let mut drained: Vec<(NonNull<[u8]>, Layout)> = Vec::new();
    let mut n = 0usize;
    loop {
        let l = Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap(); // order1
        match a.allocate(l) {
            Ok(b) => {
                drained.push((b, l));
                n += 1;
            }
            Err(_) => break,
        }
    }
    crate::expect!(n > 0, "frame drain: no blocks ever allocated");
    let l = Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap();
    crate::expect!(
        a.allocate(l).is_err(),
        "frame drain: alloc after exhaustion must fail"
    );
    for (b, l) in drained.drain(..) {
        // SAFETY: b 来自本幕 allocate、layout 同源。
        unsafe { a.deallocate(b.cast(), l) };
    }
    let l = Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap();
    // SAFETY: 复验块当轮即释。
    unsafe {
        let b = a.allocate(l).expect("stress: frame after drain");
        a.deallocate(b.cast(), l);
    }

    crate::health::report_ok(
        "stress",
        format_args!(
            "block {STEPS} mixed + {HELD} held; frame {FRAME_STEPS} mixed + {FRAME_HELD} held + drain {n} clean"
        ),
    );
}
