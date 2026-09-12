//! **并发压力/吞吐量**：`malloc-bench-rs`（Larson / mstress）× 内核分配器的
//! `GlobalAlloc` 适配器，并与 `System` 做同条件对照。
//!
//! ```sh
//! ./run.sh bench            # = cargo run --release --no-default-features --example throughput
//! ./run.sh bench -- 8 50000 # 线程数 每线程步数
//! ```
//!
//! # 为什么必须 `--no-default-features`
//!
//! `smartalloc` 的 C 层是一张无锁全局链表（见 `lib.rs` 接管段②）：本示例起多线程，
//! 而线程 spawn / 通道 / 打印都要过全局堆 —— 接管着跑会把它写坏。故这条路上不接管。
//!
//! # 这个读数**不是**正确性判据
//!
//! 吞吐量只回答"这套分流在真并发下大概什么量级、有没有塌成串行"。正确性判据在
//! `tests/mt.rs`（守恒 / 模式写读 / 池归属）与 `tests/pairing.rs`（配对账）。
//! 另：**QEMU 里的多 hart 数据不在这里** —— 那是内核侧框架档的事（本 crate 是宿主台面，
//! 它量不到真机的 hart 间竞争、锁的 cache 行抖动、TLB/SBI 代价）。
//!
//! # 适配器说明
//!
//! 内核的 `frame` / `block` 实现的是 `core::alloc::Allocator`，而 `malloc-bench-rs` 要的是
//! `GlobalAlloc`。适配器只做一件事：按 `hybrid.rs:38` 的分流点（≤ 2 KiB 走块池、否则走帧）
//! 路由，并把 `AllocError` 翻成空指针。**分流点与内核同一个数**，故量的是内核那套形状。

// 示例是**独立 crate**：内核分配器实现的是 `core::alloc::Allocator`（不稳定），
// 故这边也要开同一个 feature（lib 里开了，但集成/示例 crate 不受它覆盖）。
#![feature(allocator_api)]

use core::alloc::{Allocator, GlobalAlloc, Layout};

use alloc_probe::harness;
use alloc_probe::memory::allocator::{block, frame};
use malloc_bench_rs::{Config, Workload};

/// 内核 `hybrid.rs:38` 的分流点：≤ 2 KiB 走块池，> 2 KiB 走帧。
const SPLIT: usize = 2048;

/// 内核分配器的 `GlobalAlloc` 外壳（宿主侧，仿真 `hybrid` 的分流）。
struct KernelAlloc;

impl KernelAlloc {
    #[inline]
    fn pick(layout: Layout) -> &'static dyn Allocator {
        if layout.size() <= SPLIT {
            block::allocator()
        } else {
            frame::allocator()
        }
    }
}

// SAFETY: 两个后端都实现 `Allocator`（`allocate`/`deallocate` 的契约见 core）：
// 交付区不与其它在册区重叠、`deallocate` 用**原样的** layout 归还。对齐上界为页大小
// （帧侧只承诺页对齐），超出的请求返回空指针而不是交付一段不满足对齐的内存。
unsafe impl GlobalAlloc for KernelAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 || layout.align() > harness::PAGE {
            return core::ptr::null_mut();
        }
        match Self::pick(layout).allocate(layout) {
            Ok(buf) => buf.cast::<u8>().as_ptr(),
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: 调用方保证 (ptr, layout) 来自本分配器的一次 `alloc`。
        unsafe {
            Self::pick(layout).deallocate(
                core::ptr::NonNull::new_unchecked(ptr),
                layout,
            )
        };
    }
}

fn main() {
    let _arena = harness::boot();

    let mut args = std::env::args().skip(1);
    let threads: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let steps: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50_000);
    let cfg = Config {
        threads,
        steps_per_thread: steps,
        working_set: 256,
        mstress_blocks: 128,
    };

    println!(
        "# malloc-bench-rs：线程 {threads} / 每线程 {steps} 步 / 工作集 {} 块 / mstress {} 块",
        cfg.working_set, cfg.mstress_blocks
    );
    println!("# 分配器对照：KernelAlloc（block+frame，分流点 {SPLIT} B）vs System");
    println!(
        "{:<10} {:>14} {:>14} {:>10}",
        "workload", "kernel Mops/s", "system Mops/s", "比值"
    );
    for w in [Workload::Larson, Workload::Mstress] {
        let k = malloc_bench_rs::run(w, &cfg, || KernelAlloc);
        let s = malloc_bench_rs::run(w, &cfg, || std::alloc::System);
        println!(
            "{:<10} {:>14.2} {:>14.2} {:>9.2}×",
            format!("{w:?}"),
            k / 1e6,
            s / 1e6,
            k / s
        );
    }
    println!(
        "# 读数只回答量级（不是判据）：正确性在 tests/mt.rs 与 tests/pairing.rs；\n\
         # 真机多 hart 的数据在内核侧框架档（QEMU），本 crate 量不到。"
    );
}
