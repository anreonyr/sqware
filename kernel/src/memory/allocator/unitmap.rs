// ── 块级在位位图（debug）：任何块在被分配时必须是「无主」的 ──
//
// 页级 pageown 位图只能抓「堆页泄漏进 frame 池」；本位图按 8 字节单元追踪
// 「块当前有没有活跃持有者」——抓到**堆内部**的块级双发（同一块发给两个持有者
//  → Arc<Task> 头互写、strong 幻值）与错幂释放（如 4096 布局释放在 128B 块页）。
//
// 存储：**池区段内的固定数组区**（ARRAY_SIZE，每块区页 512 B 单元数组）——静态
// 映射、零运行时分配：flags(pa) = 数组区 + 页偏移 × 512，O(1) 无锁直查。不向
// frame 池借页（否则数组帧随页永驻而泄漏，boot selftest 帧基线即暴露）。仅在池
// inner 锁内访问（分配/释放路径已持锁），无需原子指令。
#![cfg_attr(debug_assertions, allow(dead_code))]
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::memory::PAGE_SIZE;
use crate::putln;

pub(crate) const UNITS_PER_PAGE: usize = PAGE_SIZE / 8; // 512 单元/页
const UNIT_BYTES: usize = 512; // 每页单元数组字节数

/// 块区基址（页索引基准）与数组区基址；init 写入一次，此后只读。
static POOL_BASE: AtomicUsize = AtomicUsize::new(0);
static ARRAY_BASE: AtomicUsize = AtomicUsize::new(0);
/// 数组区覆盖的块区页数（上限校验用）。
static ARRAY_PAGES: AtomicUsize = AtomicUsize::new(0);

/// 记录基准（block::init 调用恰好一次，单 hart boot 期）。
/// 数组区与块区相邻：单元数组静态映射，无需逐页分配/归还。
pub(crate) fn set(pool_base: usize, array_base: usize, array_pages: usize) {
    assert_eq!(
        array_base % PAGE_SIZE,
        0,
        "unitmap: array base must be page-aligned"
    );
    POOL_BASE.store(pool_base, Ordering::Relaxed);
    ARRAY_BASE.store(array_base, Ordering::Relaxed);
    ARRAY_PAGES.store(array_pages, Ordering::Relaxed);
}

/// 某页的单元数组指针（页须在块区内；块页必然在区内）。
fn flags(pa: usize) -> *mut u8 {
    let idx = (pa - POOL_BASE.load(Ordering::Relaxed)) / PAGE_SIZE;
    let pages = ARRAY_PAGES.load(Ordering::Relaxed);
    assert!(
        idx < pages,
        "unitmap: page {pa:#x} outside block region (idx {idx}, pages {pages})"
    );
    (ARRAY_BASE.load(Ordering::Relaxed) + idx * UNIT_BYTES) as *mut u8
}

/// 页在块区内的页序号（用于 MarkFail 报告）。
fn page_idx(pa: usize) -> usize {
    (pa - POOL_BASE.load(Ordering::Relaxed)) / PAGE_SIZE
}

/// mark 失败现场：单元值非 0（可能为 1 = 真在途，或任意字节 = 数组被写脏）。
#[derive(Clone, Copy)]
pub(crate) struct MarkFail {
    pub(crate) unit: usize,
    pub(crate) value: u8,
    pub(crate) page_idx: usize,
    pub(crate) addr: usize,
}

/// 断言 addr..addr+size 所有单元均无主（再标记为在途）。
pub(crate) fn mark(addr: usize, size: usize) -> Result<(), MarkFail> {
    let n = size / 8;
    let u0 = (addr % PAGE_SIZE) / 8;
    let flags = flags(addr);
    let page = page_idx(addr);
    for i in 0..n {
        // SAFETY: u0+n ≤ 512（块不跨页，size ≤ PAGE_SIZE）；页在块区内。
        let f = unsafe { *flags.add(u0 + i) };
        if f != 0 {
            return Err(MarkFail {
                unit: u0 + i,
                value: f,
                page_idx: page,
                addr,
            });
        }
        unsafe { *flags.add(u0 + i) = 1 };
    }
    Ok(())
}

/// 断言 addr..addr+size 所有单元均在途（再清空）。
pub(crate) fn unmark(addr: usize, size: usize) -> Result<(), MarkFail> {
    let n = size / 8;
    let u0 = (addr % PAGE_SIZE) / 8;
    let flags = flags(addr);
    let page = page_idx(addr);
    for i in 0..n {
        // SAFETY: 同 mark。
        let f = unsafe { *flags.add(u0 + i) };
        if f != 1 {
            return Err(MarkFail {
                unit: u0 + i,
                value: f,
                page_idx: page,
                addr,
            });
        }
        unsafe { *flags.add(u0 + i) = 0 };
    }
    Ok(())
}

/// 断言失败现场 dump：单元数组上下文 + 单元字节原始值。
/// **零分配**（只用 putln! 直写 + 固定缓冲）——避免 dump 触发分配器重入。
pub(crate) fn dump_fail(f: &MarkFail) {
    let page = (f.addr & !(PAGE_SIZE - 1)) as *const u8;
    let flags = flags(f.addr);
    let u0 = (f.addr % PAGE_SIZE) / 8;
    putln!(
        "[unitmap] mark/unmark fail — addr {:#x}, unit {}, value {:#x} (byte {})",
        f.addr,
        f.unit,
        f.value,
        f.value
    );
    putln!(
        "[unitmap] page {:#x}, page_idx {}, unit offset {:#x} (u0 {u0})",
        page as usize,
        f.page_idx,
        f.unit * 8
    );
    // 单元数组 ±16 字节（当前单元在中间）——逐字节直写
    let lo = f.unit.saturating_sub(16);
    let hi = (f.unit + 16).min(UNITS_PER_PAGE);
    putln!("[unitmap] array[{lo}..{hi}):");
    for i in lo..hi {
        let b = unsafe { *flags.add(i) };
        let marker = if i == f.unit { " <<<" } else { "" };
        putln!("  u{i:3} = {b:02x}{marker}");
    }
    // 页头字节（used 计数 / 整页块的 link）原始内容——逐字节直写
    putln!("[unitmap] page head:");
    for i in 0..16 {
        putln!("  b{i} = {:#02x}", unsafe { *page.add(i) });
    }
}

/// 断言整页无活跃单元——页级记账完整性检查（页永驻池区段，仅校验不归还）。
#[track_caller]
pub(crate) fn assert_page_clear(pa: usize) {
    let flags = flags(pa);
    for i in 0..UNITS_PER_PAGE {
        // SAFETY: 页在块区内，i < 512。
        let f = unsafe { *flags.add(i) };
        assert_eq!(
            f, 0,
            "block: page {pa:#x} returned with live unit {} — 计数记账错误（used-counter 提前归零）",
            i
        );
    }
}
