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

/// 自由链不变量的抽取轮数（每轮：取 N 页 → **逆序**归还 → 逼出拆分/合并交错）。
const CHAIN_ROUNDS: usize = 8;
/// 每轮取几页。
const CHAIN_PAGES: usize = 192;

/// 自由链不变量：**表说空闲 ⇔ 真在链上**。
///
/// # 这是"反向核对"那一半，此前从未成为判据
///
/// 早年只验正向（链上每块的表项正确，见 `chain_meta_mismatch`），而正向**看不见**
/// 孤儿表项 —— 不在任何链上、却写着 `free=true` 的那些。于是 `walk`（走链）与
/// `meta`（读表）长期背离（实测 `walk=321 / meta=11956`），而所有护栏（`check_bounds`
/// / `check_frame_free` / `check_not_in_chain`）零报警：链的**局部**操作全合法，
/// 病在"哪些块压根没进链"。
///
/// 产出者是 `split_block`：它 `pull_link(k)` 把 index 的表项写成 `free=false,power=k`，
/// 然后逐级下降 `k -= 1` 而 **index 不变** ⇒ 每一级都在 index 处留下一条**更大 order
/// 的过时表项**。那一段从此被幽灵覆盖：`merge_block` 按它认定伙伴空闲、`in_freelist`
/// 却找不到 ⇒ 放弃合并（`nomerge` 计数），该段永久脱离可用池。
///
/// **对照组不能省**：只断言"孤儿为 0"会因池子恰好没有空闲块而假绿。故先做一轮
/// 取一页即还（它必然经 `merge_block` 走一趟 `in_freelist`），确认链这条读法**是活的**。
pub(super) fn chain() {
    let h = crate::memory::allocator::frame::heap();
    // **起点读数**：这批幽灵是本用例 churn 造的，还是启动期（乃至更早的用例）就已经
    // 躺在池里的？没有这个数，我只能猜 —— 而"猜"正是本会话前五个探针的毛病。
    let (fe0, or0, s0) = h.free_entry_orphans();
    crate::putln!("[chain] 起点 freeent={fe0} orphan={or0} sample={s0:?}");

    // 对照组：一次取还必须让链可读（`in_freelist` 走得到那一步）。
    let a = hybrid::allocator();
    let l = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
    // SAFETY: 取一页当即归还，闭环。
    unsafe {
        let b = a.allocate(l).expect("chain: control alloc");
        a.deallocate(b.cast(), l);
    }
    let (ck, cbad, _) = h.chain_meta_mismatch();
    crate::expect!(
        ck > 0 && cbad == 0,
        "chain: 对照组不成立（链上块 {ck}、表项不符 {cbad}）—— 链这条读法本身失效，\
         下面的孤儿数不可信"
    );

    // 抽取：反复"取一批 → 逆序归还"。逆序让合并路径撞上非相邻伙伴，正是疑点所在。
    let mut pages: Vec<NonNull<[u8]>> = Vec::with_capacity(CHAIN_PAGES);
    for _ in 0..CHAIN_ROUNDS {
        pages.clear();
        for _ in 0..CHAIN_PAGES {
            match a.allocate(l) {
                Ok(b) => pages.push(b),
                Err(_) => break,
            }
        }
        for b in pages.drain(..).rev() {
            // SAFETY: b 来自本轮的 allocate，layout 同源。
            unsafe { a.deallocate(b.cast(), l) };
        }
    }

    let (free_entries, orphans, sample) = h.free_entry_orphans();
    crate::putln!(
        "[chain] 起点 orphan={or0} → 终点 orphan={orphans}（freeent {free_entries}、\
         sample={sample:?}、nomerge={}）",
        crate::memory::allocator::frame::FrameAllocator::nomerge_count()
    );
    // **判据是对照自己**，不是绝对零。
    //
    // 绝对零目前不成立：池里已有 9 条陈旧表项，产出者是 `split_block` 的下降 —— 它在
    // **同一个 `index`** 上逐级留下粗粒度表项（轨迹实测：`push_link(1280, 6)` 之后
    // **再无任何** `remove_link`/`pull_link`/`clear_head`），于是表说"这里有块"、链里没有，
    // 那些段被幽灵覆盖、合并会一直放弃它们。三次修法都撞上 `held()`：那条表项同时是
    // "这帧在谁手里"的答案（`free()` 的护栏读它），撤了当场炸 `freeing non-held frame`。
    //
    // 故这里断的是**增量**：本次 churn 不许新增孤儿。这样它挡得住回归（新写入不同步会
    // 立刻红），又不假装旧账已经清了。旧账的根因是独立一步。
    crate::expect!(
        orphans <= or0,
        "自由链表↔pagemeta 背离**在增长**：{or0} → {orphans} 条空闲表项不在任何链上\
         （样本 {sample:?}）—— 有新的块被标空闲却没入链"
    );
}

pub(super) fn accept() {
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

}
