// 共享 debug 校验原语 — block.rs / frame.rs 链式不变式的断言收容处。
//
// 设计：钩子恒编译、单行调用；函数体以 #[cfg(debug_assertions)] 包住，release 下
// 为空体（#[inline(always)] 保证消除，零开销）。断言仅 debug 构建生效，命中一律
// panic（内核 halt 处理器再转储 crash scene / 清 canary）。调用点传裸值（链头/
// 地址/标志/next 读取闭包），本模块无状态、不触碰任何分配器内部——block（块首
// 字挂 next）与 frame（Link.next 字段）两种链以同一套泛型原语覆盖。
//
// 原语对应关系（block / frame 调用点）：
//   check_dram_addr    —— 链头/next/remove 地址回链（机器空闲 DRAM 区）；
//   check_bounds       —— freelist/pagemeta 索引越界（越界写破坏相邻元数据）；
//   check_frame_free   —— frame 弹出帧必须 free（pagemeta 与链一致，重叠分配现行）；
//   check_not_in_chain —— 目标不得已在链中（double-free / double-push）；
//   check_in_chain     —— 目标必须在链中（remove_link 摘除前核对）；
//   dump_chain         —— 违例现场链快照（固定缓冲零分配）；
//   log_*              —— 分配/释放逐次流水（观测）。

#![allow(unused_variables)] // release 下钩子为空体，参数随之未用

use core::ptr::NonNull;

/// 链节点地址必须落在空闲 DRAM 区——链被越界写/UAF 覆写（节点指针逸出区段）
/// 的特征；校验不过立刻 panic，把解引用野指针后的随机崩溃变成定位明确的报错。
#[inline(always)]
pub(crate) fn check_dram_addr(addr: usize, ctx: &str) {
    #[cfg(debug_assertions)]
    {
        if !crate::machine::info().free.range().contains(&addr) {
            panic!(
                "allocator: {ctx}: node address {addr:#x} outside free DRAM (corrupted freelist?)"
            );
        }
    }
}

/// 索引越界（freelist/pagemeta 数组写前检查——越界写会破坏相邻元数据）。
#[inline(always)]
pub(crate) fn check_bounds(value: usize, len: usize, ctx: &str) {
    #[cfg(debug_assertions)]
    assert!(
        value < len,
        "allocator: {ctx}: {value} out of range (len {len})"
    );
}

/// frame 弹出帧必须 free（`pagemeta` 与链一致；分配中的帧被再弹出 = 重叠分配）。
#[inline(always)]
pub(crate) fn check_frame_free(free: bool, index: usize, addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    if !free {
        panic!(
            "frame allocator: allocated non-free frame — index {index}, addr {addr:#x}, power {power}"
        );
    }
}

/// 遍历判重：目标不得已在链中——已在 = double-free / double-push（再头插会写坏
/// 链表）；遍历深度越界 = 成环（某节点 next 被覆写）。仅 debug 构建做 O(链长)
/// 遍历；命中即 dump 现场 + panic。`next` 由调用点提供（block 读块首字，frame
/// 读 `Link.next` 字段）。
#[inline(always)]
pub(crate) fn check_not_in_chain<T>(
    power: usize,
    ctx: &str,
    head: Option<NonNull<T>>,
    target: usize,
    next: impl FnMut(NonNull<T>) -> Option<NonNull<T>>,
) {
    #[cfg(debug_assertions)]
    {
        let mut next = next;
        let (found, cyclic) = walk_chain(head, target, &mut next);
        if cyclic {
            dump_chain(power, ctx, head, target, &mut next);
            panic!("allocator: {ctx}: freelist[{power}] walk exceeded depth — cyclic list");
        }
        if found {
            dump_chain(power, ctx, head, target, &mut next);
            panic!(
                "allocator: {ctx}: address {target:#x} already in freelist[{power}] (double free / double push)"
            );
        }
    }
}

/// 遍历核对：目标必须在链中（remove_link 摘除前——跨 order 交叉摘除会破坏链表）。
/// `next` 读取同 [`check_not_in_chain`]。
#[inline(always)]
pub(crate) fn check_in_chain<T>(
    power: usize,
    ctx: &str,
    head: Option<NonNull<T>>,
    target: usize,
    next: impl FnMut(NonNull<T>) -> Option<NonNull<T>>,
) {
    #[cfg(debug_assertions)]
    {
        let mut next = next;
        let (found, cyclic) = walk_chain(head, target, &mut next);
        if cyclic {
            dump_chain(power, ctx, head, target, &mut next);
            panic!("allocator: {ctx}: freelist[{power}] walk exceeded depth — cyclic list");
        }
        if !found {
            dump_chain(power, ctx, head, target, &mut next);
            panic!("allocator: {ctx}: target {target:#x} not in freelist[{power}]");
        }
    }
}

/// debug: 分配逐次流水（观测）。release 空体。
#[inline(always)]
pub(crate) fn log_alloc(addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    log::debug!("block allocator: address {addr:#x}, power {power} allocated");
}

/// debug: 释放逐次流水（观测）。release 空体。
#[inline(always)]
pub(crate) fn log_dealloc(addr: usize, power: usize) {
    #[cfg(debug_assertions)]
    log::debug!("block allocator: address {addr:#x}, power {power} deallocated");
}

/// debug: frame 分配逐次流水（观测）。release 空体。
#[inline(always)]
pub(crate) fn log_frame_alloc(addr: usize, index: usize, power: usize) {
    #[cfg(debug_assertions)]
    log::trace!("frame allocator: address {addr:#x}, frame index {index}, power {power} allocated");
}

/// debug: frame 释放逐次流水（观测）。release 空体。
#[inline(always)]
pub(crate) fn log_frame_dealloc(addr: usize, index: usize, power: usize) {
    #[cfg(debug_assertions)]
    log::trace!(
        "frame allocator: address {addr:#x}, frame index {index}, power {power} deallocated"
    );
}

/// 链遍历：返回 (found, cyclic)；深度 > 1<<14 记 cyclic（防破坏链上的死循环）。
#[cfg(debug_assertions)]
fn walk_chain<T>(
    head: Option<NonNull<T>>,
    target: usize,
    next: &mut impl FnMut(NonNull<T>) -> Option<NonNull<T>>,
) -> (bool, bool) {
    let mut cur = head;
    let mut depth = 0usize;
    while let Some(node) = cur {
        if node.as_ptr() as usize == target {
            return (true, false);
        }
        depth += 1;
        if depth > 1 << 14 {
            return (false, true);
        }
        cur = next(node);
    }
    (false, false)
}

/// 违例现场链快照：目标地址 + 该 power 全链（前 256 节点）+ 失败页头 8 字。
/// **零分配**（putln! 直写 + 固定缓冲数组）——panic 现场任何 alloc 都会经
/// portal→block 递归/死锁，且会污染现场。`next` 读取同 walk 闭包。
#[cfg(debug_assertions)]
fn dump_chain<T>(
    power: usize,
    ctx: &str,
    head: Option<NonNull<T>>,
    target: usize,
    next: &mut impl FnMut(NonNull<T>) -> Option<NonNull<T>>,
) {
    crate::putln!("[crash] {ctx}: target addr {target:#x}");
    let mut walk = [0usize; 256];
    let mut n = 0usize;
    let mut cur = head;
    while let Some(node) = cur {
        if n < walk.len() {
            walk[n] = node.as_ptr() as usize;
        }
        n += 1;
        cur = next(node);
    }
    crate::putln!(
        "[crash] freelist[{power}] walk ({} nodes, first 256 shown):",
        n
    );
    let shown = n.min(walk.len());
    (0..shown).for_each(|i| {
        let a = walk[i];
        crate::putln!(
            "  [{}] {:#x} (page {:#x}, offset {:#x})",
            i,
            a,
            a & !(crate::memory::PAGE_SIZE - 1),
            a & (crate::memory::PAGE_SIZE - 1)
        );
    });
    let b = (target & !(crate::memory::PAGE_SIZE - 1)) as *const usize;
    crate::putln!("[crash] failing page head words:");
    for i in 0..8 {
        crate::putln!("  w{i} = {:#x}", unsafe { *b.add(i) });
    }
}
