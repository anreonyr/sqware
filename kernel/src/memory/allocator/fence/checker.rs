// 护栏层 · checker — 分配器链式不变式的断言收容处。
//
// 钩子恒编译、单行调用；命中一律 panic（halt 处理器再转储 crash scene）。调用点传裸值，
// 本模块无状态、不触碰任何分配器内部。门分两档（见下"为什么 audit 档也要跑"）：
//   O(1) 的检查（dram/bounds/frame_free/frame_held）→ debug 档 **与** audit 档都编译
//   O(链长) 的链式遍历（not_in_chain / in_chain）与 log_* → 只 debug 档
// 其余档空体零开销。

#![allow(unused_variables)] // release 下钩子为空体，参数随之未用

use core::ptr::NonNull;

/// 链节点地址必须落在空闲 DRAM 区——链被越界写/UAF 覆写（节点指针逸出区段）
/// 的特征；校验不过立刻 panic，把解引用野指针后的随机崩溃变成定位明确的报错。
#[inline(always)]
pub(crate) fn check_dram_addr(addr: usize, ctx: &str) {
    #[cfg(any(debug_assertions, feature = "audit"))]
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
    #[cfg(any(debug_assertions, feature = "audit"))]
    assert!(
        value < len,
        "allocator: {ctx}: {value} out of range (len {len})"
    );
}

/// frame 弹出帧必须 free（`pagemeta` 与链一致；分配中的帧被再弹出 = 重叠分配）。
#[inline(always)]
pub(crate) fn check_frame_free(free: bool, index: usize, addr: usize, power: usize) {
    #[cfg(any(debug_assertions, feature = "audit"))]
    if !free {
        panic!(
            "frame allocator: allocated non-free frame — index {index}, addr {addr:#x}, power {power}"
        );
    }
}

/// frame 释放帧必须**仍在手**（`pagemeta` 说它是某块的块首且 non-free）——
/// 双释放 / 释放陌生页的特征。O(order) 无锁内遍历，故与 `check_frame_free`
/// 一同进 audit 档（banker 的 `credit` 删掉后由它接位）。
// 整函数与调用点同 gate：它的实参是一次 O(order) 的 pagemeta 遍历，产品档不该付。
#[cfg(any(debug_assertions, feature = "audit"))]
#[inline(always)]
pub(crate) fn check_frame_held(held: bool, index: usize, addr: usize, power: usize) {
    if !held {
        panic!(
            "frame allocator: freeing non-held frame — index {index}, addr {addr:#x}, power {power}"
        );
    }
}

/// 释放的帧必须是**块首**，且大小与它在手时一致。
///
/// # 定案：命中数曾达 1811 次，**全部是粗表项造成的误判**（已修，现恒为 0）
///
/// `check_frame_held` 只问"这一页在不在手"，而"在手"的判据是 `held(pa)` —— 它按
/// **覆盖**回答。而旧版 `split_block` 会按**取出时的桶号**写块首表项（`power=k`），
/// 那条"粗表项"覆盖了拆分推回的伙伴帧 ⇒ 释放一个**邻居**（自己那块的首帧）时，
/// `interior_of_held` 会命中那条粗表项，报成"释放中间帧"。
///
/// 取证：`[conserve]` 的守恒核对给出 `表在手 − 账在手 = +528 帧`，而**恰好等于**同一
/// 快照里"跨度内空闲帧"的 528 —— 即那条粗表项把已归还的伙伴帧仍算作在手。粗表项在
/// 源头去掉后：`interior` 命中 **1811 → 0**、`covered` 写点 **31376 → 0**、
/// `表在手 = 账在手 = 735`（两口径逐帧相等）。
///
/// 故这一条**保留**（它仍是"调用者拿小块 layout 释放大块内部帧"的直接入口，真发生时
/// 会破坏 `表在手 = 账在手`），但当前读数应为 0 —— 框架档用例对它断绝对零。
/// `#[track_caller]`：panic 位置须落在**调用者**那一行（否则只指到本函数，等于没给
/// 位置 —— 实测踩过：`checker.rs:86` 这种读数指不出是谁在释放中间帧）。
#[cfg(any(debug_assertions, feature = "audit"))]
#[track_caller]
#[inline(always)]
pub(crate) fn check_frame_head(
    interior: Option<(usize, u8)>,
    index: usize,
    addr: usize,
    power: usize,
) {
    if let Some((base, bpower)) = interior {
        // 只累加一个计数。**采样与调用者回溯（`caller_site`）已删**：那套东西是为
        // "1811 次到底是谁在放中间帧"取证用的，而那个问题已结案 —— 1811 次全部是
        // `split_block` 粗表项造成的**误判**（修后恒为 0），采样窗口再也不会打开。
        // 判据本身留着（见本函数文档）：它是"调用者拿小块 layout 释放大块内部帧"
        // 的唯一入口，而那种事一旦发生就会破坏 `表在手 = 账在手`。
        let _ = (base, bpower, power);
        INTERIOR_FREES.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
    }
}

/// **判据**：释放"在手块的中间帧"的累计次数 —— 必须恒为 0。
///
/// 这是**沉默的损坏**：分配器按帧处理，把中点当 `power` 大小的块入链 ⇒ `pagemeta` 里
/// 长出"在手块跨度内的表项"。现有护栏放行它，因为 `check_frame_held` 问的是"在不在手"，
/// 而"在手"的判据 `held(pa)` 按**覆盖**回答 —— 中间帧照样答"在手"。
///
/// # 为什么是计数而不是 panic（这一条现在有了新的理由）
///
/// 历史读数（启动期 1811 次）曾指向"调用者在放中间帧"，但守恒核对证明那是**粗表项的
/// 误判**（详见 [`check_frame_head`]）。判据仍留着：真发生"拿小块 layout 释放大块内部帧"
/// 时，它会立刻破坏 `表在手 = 账在手` 这条跨账恒等式。
///
/// 保留计数而非 panic 的理由：这条判据的**归属**已经交给了框架档用例
/// （`[conserve]` 裁决 + `interior2 == 0` 的绝对零断言），那里红掉能指名道姓；
/// 而在产品档把它升成 panic，等于拿一次误判换整机停机。
pub(crate) static INTERIOR_FREES: ::core::sync::atomic::AtomicUsize =
    ::core::sync::atomic::AtomicUsize::new(0);


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
/// **零分配**（putln! 直写 + 固定缓冲数组）——panic 现场任何 alloc 都会重入
/// 分配器锁（inner/tally/frame）递归/死锁，且会污染现场。`next` 读取同 walk 闭包。
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
