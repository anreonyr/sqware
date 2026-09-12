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

    // **自洽约束**：两条独立扫表法必须给出同一个数。
    //
    // 每条表项都声明自己是块首（占 `2^power` 帧），所以"按块首步进扫"与"逐条数"**必须相等**。
    // 实测不等，且差得极远：步进法 **54**、逐条法 **197** —— 也就是**143 条表项落在
    // 他者声明的跨度里**（块内部），而错位数为 0（每条自己对自身大小都是对齐的）。
    //
    // 这条比"孤儿数"更根本：此前所有"表 vs 链"的对质都在拿这 197 条里的**不同子集**比较，
    // 于是两个探针互相矛盾（`freeent=32` 而逐 order 扫表求和得 34）——**缺自洽约束**正是
    // 本会话前五个探针全部落空的那个毛病。这条约束一加，内部不一致立刻无处可藏。
    let (stepped, flat, misaligned, first_bad) = h.scan_disagree();
    crate::putln!(
        "[scan] 表项：步进法={stepped} 逐条法={flat}（差 {}）错位={misaligned} 首个错位={first_bad:?}",
        flat as i64 - stepped as i64
    );
    // 与孤儿那条同形：**断增量，不断绝对值**。绝对值目前很大（差 143），旧账的根因尚未
    // 修；但"本次 churn 不许把这个差推大"是能立刻立的判据 —— 新写入一处不同步即红。
    let gap0 = flat as i64 - stepped as i64;
    let covered0 = crate::memory::allocator::frame::FrameAllocator::covered_writes();
    let interior0 = crate::memory::allocator::frame::FrameAllocator::interior_frees();

    let census = h.free_block_census();
    let mut first_off = 0usize;
    for (o, (meta_heads, chain)) in census.iter().enumerate() {
        if *meta_heads != *chain {
            first_off += 1;
            if first_off <= 4 {
                crate::putln!("[census] p={o} 表说空闲块首={meta_heads} 链上={chain}");
            }
        }
    }
    let (st2, fl2, _mis2, _fb2) = h.scan_disagree();
    let (free_entries, orphans, sample) = h.free_entry_orphans();
    crate::putln!(
        "[chain] 起点 orphan={or0} → 终点 orphan={orphans}；表项差 {gap0} → {}（步进 {stepped}→{st2}、\
         逐条 {flat}→{fl2}）；freeent {free_entries}、sample={sample:?}、nomerge={}",
        fl2 as i64 - st2 as i64,
        crate::memory::allocator::frame::FrameAllocator::nomerge_count()
    );
    let covered2 = crate::memory::allocator::frame::FrameAllocator::covered_writes();
    // 直接对有案底的样本取证：17397 是 17396 那块（power=1）的中间帧吗？
    for probe in [512usize, 17397, 1280, 1344] {
        let pa = h.frame_addr_of(probe);
        crate::putln!("[interior] idx={probe} pa={pa:#x} => {:?}", h.interior_of_held(pa));
    }
    let bd = crate::memory::allocator::frame::FrameAllocator::covered_breakdown();
    crate::putln!(
        "[covered] 累计 {covered0} → {covered2}；分类 push→free={} push→held={} pull→free={} pull→held={} clear→free={} 其它={}",
        bd[0], bd[1], bd[2], bd[3], bd[4], bd[5]
    );
    let interior2 = crate::memory::allocator::frame::FrameAllocator::interior_frees();
    let (lossy, aliased) = crate::memory::allocator::frame::FrameAllocator::interior_split();
    crate::putln!(
        "[interior] 释放在手块中间帧 {interior0} → {interior2}；其中丢帧(power<bpower)={lossy}、块首误判={aliased}"
    );
    crate::expect!(
        interior2 <= interior0,
        "释放中间帧的次数在增长：{interior0} → {interior2} —— 分配器按帧处理，会把中点当块首入链"
    );
    crate::expect!(
        covered2 <= covered0,
        "写点判据在增长：{covered0} → {covered2} —— 有新的索引被写进**别人已声明的跨度**里\
         （块首之间不可能互相包含）"
    );
    crate::expect!(
        fl2 as i64 - st2 as i64 <= gap0,
        "pagemeta 的自我不一致在增长：表项差 {gap0} → {}（步进 {st2}、逐条 {fl2}）—— \
         有新的表项落进别条声明的跨度里",
        fl2 as i64 - st2 as i64
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
