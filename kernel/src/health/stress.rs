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

/// **守恒快照单行组**：把 [`Conserve`] 的读数摊成 5 行（`[conserve/<tag>]` 前缀可 grep）。
///
/// 为什么把 20 个数一次摊开而不是只印"结论"：本会话的教训是**分次读、只读结论**
/// 必然读出一个看似有信息量的故事。摊开后任何一步算错都看得见（例如 `总` 与四项
/// 之和对不上、`无主` 不为 0）。
fn dump(tag: &str, c: &crate::memory::allocator::frame::Conserve) {
    crate::putln!(
        "[conserve/{tag}] 总 {} = 在手 {} + 空闲 {} + 洞 {} + 无主 {}（账 {}-{}={}）",
        c.total,
        c.held,
        c.idle,
        c.holes,
        c.unaccounted(),
        c.taken,
        c.given,
        c.taken as i64 - c.given as i64
    );
    crate::putln!(
        "[conserve/{tag}] 链 walk={} 表空闲={} 差={} 跨度内空闲帧={} 残差={}（链上不符={}）",
        c.walk,
        c.idle,
        c.chain_gap(),
        c.inner_free_frames(),
        c.residual(),
        c.chain_bad
    );
    crate::putln!(
        "[conserve/{tag}] 账↔表：表在手={} vs 账在手（累计分配−累计释放）={} ⇒ 差={}",
        c.held,
        c.taken as i64 - c.given as i64,
        c.ledger_gap()
    );
    crate::putln!(
        "[conserve/{tag}] 跨度内表项：在手内空闲 {}/{} 帧、在手内在手 {}/{} 帧、\
         空闲内空闲 {}/{} 帧、空闲内在手 {}/{} 帧",
        c.ghost_free.0,
        c.ghost_free.1,
        c.nested_held.0,
        c.nested_held.1,
        c.nested_free.0,
        c.nested_free.1,
        c.held_in_free.0,
        c.held_in_free.1
    );
    crate::putln!(
        "[conserve/{tag}] 表项 步进={} 逐条={} 腐化={}；未入链 落表项 {}/{} 帧、跨度内 {}/{} 帧；链节点={}",
        c.stepped,
        c.flat,
        c.bad_entries,
        c.orphan_entries,
        c.orphan_frames,
        c.inner_orphan_entries,
        c.inner_orphan_frames,
        c.chain_nodes
    );
}

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
/// # 三个产出者（已全部修掉，读数归零）
///
/// 1. **粗表项**：`split_block` 曾按**取出时的桶号**写 `index` 的表项（`free=false,
///    power=k`），而它每降一级就把伙伴推回空闲桶 ⇒ 那条表项覆盖了已经空闲的伙伴。
///    修法：`pull_link` 只撤不立，拆分收尾按**最终 order** 写一次（O(1)）。
/// 2. **静默失效的指针写**：`push_link`/`pull_link` 里 `x.read().prev = …` 是对
///    `NonNull::read()` 返回的**临时副本**赋值 —— 编译通过、不报错，于是双向链表的
///    `prev` 从未维护；而 `remove_link` 正是用 `prev == None` 判"我是桶头"，遂在摘除
///    链中间节点时把**真正的桶头**覆盖成陈旧 `next`，链头那几个块从此不可达却仍留着
///    `free=true`（实测 11 条 / 622 帧）。修法：`(*x.as_ptr()).prev = …`。
/// 3. **为补偿 ① 加的"跨度清扫"**：`push_link` 曾遍历 `2^power` 个槽清掉跨度内的空闲
///    表项。① 在源头修好后它零命中（且每次入链都要扫 `2^power` 槽），已删。
///
/// # 判据
///
/// 修好后四条平衡**恰好为 0**（见 [`dump`] 的 `[conserve]` 行与 `conserved()`）：
/// 步进=逐条、链=空闲、表在手=账在手、未入链=0，另有逐环核对双向链表
/// （`chain_audit`）—— 那条是第 ② 类缺陷唯一能在当场抓住的判据。
///
/// **对照组不能省**：只断言"孤儿为 0"会因池子恰好没有空闲块而假绿。故先做一轮
/// 取一页即还（它必然经 `merge_block` 走一趟 `in_freelist`），确认链这条读法**是活的**。
pub(super) fn chain() {
    let h = crate::memory::allocator::frame::heap();
    // **起点读数**：链↔表的背离是本次 churn 造的，还是启动期就已经躺在池里的？
    // 两点对照才判得了"本次 churn 有没有引入背离"；单点读数说明不了任何事。
    let (fe0, or0, s0) = h.free_entry_orphans();
    crate::putln!("[chain] 起点 freeent={fe0} orphan={or0} sample={s0:?}");
    // **守恒底账**：churn 之前先记全，churn 之后再记一次 —— 只有两点对照才能判
    // "本次 churn 有没有丢帧/重复登记"，单点读数说明不了任何事。
    let c0 = h.conserve();
    dump("起点", &c0);

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
    // **孤儿的形态**：节点是"被摘掉了两侧"（写侧漏配对）还是"结构完好却走不到"
    // （某次 remove_link 按过期 Link 摘错了节点）？两者修法相反，先把形态量出来。
    let (shape, tally) = h.orphan_shape();
    crate::putln!(
        "[orphan] 形态：孤立={} 结构完好={} 结构不一致={}；样本 (idx, power, prev, next, next回指, prev正指)={shape:?}",
        tally[0],
        tally[1],
        tally[2]
    );
    crate::putln!(
        "[merge] 放弃合并 nomerge={}；MR(bound,meta,chain,ok)={:?}",
        crate::memory::allocator::frame::FrameAllocator::nomerge_count(),
        crate::memory::allocator::frame::FrameAllocator::merge_census()
    );
    // **逐环核对双向链表**：这条是那处"静默失效的指针写"唯一能在当场抓住的判据
    // （详见 `FrameInner::chain_audit`）。修前 `prev` 从不维护，症状要等上千次操作
    // 之后才以"桶头被覆盖"的形式出现。
    let (la_nodes, la_cycles, la_bad, la_first) = h.chain_audit();
    crate::putln!(
        "[linked] 逐环核对：节点={la_nodes} 环={la_cycles} 不一致={la_bad} 首个={la_first:?}"
    );
    crate::expect!(
        la_bad == 0 && la_cycles == 0,
        "双向链表结构不一致：环 {la_cycles}、不符 {la_bad}（首条 {la_first:?}）—— \
         取链节点的 `prev`/`next` 与前后邻居对不上；`remove_link` 会据此走错分支、\
         把真正的桶头覆盖掉（历史读数：8 次覆盖、11 条表项/622 帧从此不可达）"
    );
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

    // ── 守恒裁决 ──
    //
    // 这一组是**唯一**能判定那对矛盾的读数："1811 次释放中间帧（每次 ≥1 帧，账上该少
    // ~7 MiB）"与"boot / 用例无恙"不能同时为真。三条平衡各有分工（判据现为绝对零）：
    //
    //   · `无主`（(S) 式）≠ 0 ⇒ **帧从两个账上都消失了**（既不在手也不空闲），真丢帧；
    //   · `链 − 空闲 − 跨度内空闲`（(C) 式）收不平 ⇒ 还有没建模的机制，读数不可用；
    //   · `链 − 空闲`（`chain_gap`）**的符号**是判词：正 ⇒ 那些帧在链里（重复登记，
    //     池子不缺内存、但同一帧被两处登记，危险）；负 ⇒ 表说空闲而链上找不到，真丢。
    let c1 = h.conserve();
    dump("终点", &c1);
    crate::putln!(
        "[conserve] 裁决：无主 {} → {}；链−空闲 {} → {}；残差 {} → {}",
        c0.unaccounted(),
        c1.unaccounted(),
        c0.chain_gap(),
        c1.chain_gap(),
        c0.residual(),
        c1.residual()
    );
    crate::expect!(
        c1.unaccounted() == 0,
        "有帧从两个账上同时消失：无主 {} 帧（既不在手、也不空闲、也不是保留区）—— \
         `clear_head` 撤表项留下的洞无人接管；这些帧再也分配不出去",
        c1.unaccounted()
    );
    crate::expect!(
        c1.residual() == 0 && c0.residual() == 0,
        "守恒模型不完整：残差 {} → {}（起点就非 0 说明我的口径本身有漏，\
         两端读数都不可用）—— 先修模型，别拿它讲结论",
        c0.residual(),
        c1.residual()
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
    // ── 断**绝对零**（不再断增量）──
    //
    // 这一组判据此前只能断增量：池里躺着 11 条陈旧空闲表项（622 帧）、528 帧被粗表项
    // 重复算作在手、`nomerge` 28 次、`covered` 写点 31376 次，根因是三处——
    //   ① `split_block` 按**取出时的桶号**写块首表项（粗表项覆盖了刚推回的伙伴）；
    //   ② `push_link`/`pull_link` 里 `x.read().prev = …` 是对**临时副本**赋值，
    //      双向链表的 `prev` 从未维护 ⇒ `remove_link` 依据 `prev == None` 判"我是桶头"
    //      时走错分支，把真正的桶头覆盖成陈旧 `next`，链头那些块从此不可达却仍
    //      留着 `free=true`；
    //   ③ 为补偿 ① 而在 `push_link` 里加的"跨度清扫"。
    // 三处修好后全部指标**恰好为 0**（步进=逐条 178=178、链=空闲 14128=14128、
    // 表在手=账在手 735=735、orphan=0、nomerge=0、covered=0）。故判据升格为绝对零：
    // 任何一条非零都意味着"表 ↔ 链"重新开始说两套话——那类背离先前正是以
    // "分配看着正常、偶尔 OOM"的面目出现的。
    crate::expect!(
        orphans == 0,
        "自由链表↔pagemeta 背离：{orphans} 条空闲表项不在任何链上（样本 {sample:?}）——\
         有块被标空闲却没入链（或入了链却从桶头走不到），那些帧再也分配不出去"
    );
    crate::expect!(
        c1.walk == c1.idle,
        "链↔表不等：走链 {} 帧 vs 表说空闲 {} 帧（差 {}）—— 链的成员集合与\
         `pagemeta` 的空闲集合必须逐帧一致",
        c1.walk,
        c1.idle,
        c1.chain_gap()
    );
    crate::expect!(
        c1.chain_bad == 0,
        "链上有 {} 个节点与表项不符（表项缺失 / 不空闲 / 幂次与桶号不符 / 越界）",
        c1.chain_bad
    );
    crate::expect!(
        c1.ledger_gap() == 0,
        "账↔表不等：表说在手 {} 帧、账（累计分配−累计释放）{} 帧（差 {}）——\
         两者是同一件事的两份账，不等即为记账缺陷",
        c1.held,
        c1.taken as i64 - c1.given as i64,
        c1.ledger_gap()
    );
    crate::expect!(
        fl2 == st2,
        "pagemeta 自我不一致：步进法 {st2} 条 vs 逐条法 {fl2} 条 —— 表项落在别条声明的跨度里"
    );
    crate::expect!(
        covered2 == 0,
        "写点判据非零：{covered2} 次写进**别人已声明的跨度**（块首之间不可能互相包含）"
    );
    crate::expect!(
        interior2 == 0,
        "释放\"在手块中间帧\" {interior2} 次 —— 分配器按帧合并会把中点当块首入链"
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
