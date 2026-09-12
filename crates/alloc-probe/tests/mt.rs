//! 多线程（扩展）—— 并发压力下的分配器判据。
//!
//! 与单线程那两层（`tests/pairing.rs` / `tests/heap.rs`）的分工：那两层判"分配器**自己**
//! 的宿主账"，这一层判"**共享状态**在真并发下守不守得住"——块池的 per-hart 归属、泵
//! （`feed`/`suck`）的回路、buddy 的合并链、以及簿记表的复合步。
//!
//! # 判据（每条都有对应通道，不靠"跑起来没崩"）
//!
//! | 判据 | 通道 |
//! |---|---|
//! | 交付区跨线程不重叠 / 不被簿记踩 | 交付区**模式写读**（共享内存，两人写不同 tag ⇒ 归还时必有一方不符）+ 原子登记表（尽力而为的辅助） |
//! | 取还守恒 | `statistics` 原子账：**块级严格相等**；帧级/池级只差池迟滞（≤ 9 页） |
//! | 在途块全部归还 | `concurrent::registry_live() == 0` |
//! | per-hart 池真的分家 + 跨 hart 归还回本池 | `block::heap().own(pa)` 池归属读数 |
//! | 本层**自己的**对象不漏 | `dropcount`（读数归因的前提：台面漏对象，LSan/dhat 的读数就不能归给分配器） |
//!
//! # 为什么这份文件在 `smartalloc` 档下不编进来
//!
//! `smartalloc` 的 C 层是一张**无锁全局链表**（`lib.rs` 接管段②）：多线程并发 alloc/free
//! 会把它写坏。要跑这层就得关掉接管（`./run.sh mt` 给的就是 `--no-default-features`）。
//! `mockalloc`（线程局部账）与 `dhat`（全局锁 + 进程级读数）本身是线程安全的，故它们档下
//! 这层照跑 —— 只是那两档的读数不参与本层的判据。

#![cfg(all(not(feature = "smartalloc"), not(miri)))]
// Miri 下**不编进来**（裁决，不是省事）：Miri 在这套负载上要跑几十分钟到小时级 ——
// 16 MiB 台面 + 千字节级交付区模式写读，解释执行 × 向量时钟（实测：全量 miri 20 分钟
// 没跑完，已叫停）。分工是明确的：**Miri 查 UB**（裸指针来去、交付区写读 —— 由单线程
// 那几条 `cfg(miri)` 缩规模的用例承担，秒级）；**并发竞争由 TSan 承担**（`./run.sh tsan`，
// `-Zbuild-std`，秒级，实测零报告）。两条路各管一段，别互相顶替。
// 集成测试是**独立 crate**：内核分配器源码用的是 `core::alloc::Allocator`（不稳定），
// 故这边也要开同一个 feature（lib 里已开，集成测试不受它的 `#![feature]` 覆盖）。
#![feature(allocator_api)]

use core::alloc::Layout;
use core::ptr::NonNull;

use alloc_probe::concurrent;
use alloc_probe::harness;
use alloc_probe::machine;
use alloc_probe::memory::allocator::block;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

/// 并发线程数（= 4 个 hart 时，两个线程共用一池 ⇒ `push` 与 `feed` 两条路都走到）。
const THREADS: usize = 4;
/// 每线程的计划长度（每条动作 2 字节）。
#[cfg(not(miri))]
const PLAN_BYTES: usize = 160;
/// Miri 下缩小 8 倍：解释执行 + 数据竞争检测，规模要压到秒级。
#[cfg(miri)]
const PLAN_BYTES: usize = 8;

/// **per-hart 池与跨 hart 回路**：hart 0 取块 → hart 1 归还（`feed`）→ hart 0 再取，
/// 必须仍拿到归属**自己池**的块（本池 `pull` 先 `suck`，把泵里的块抽回池）。
///
/// 这条用例一次钉住三件事：
/// ① 两个 hart 的块归属**不同**池（per-hart 池真的分家，不是"都塞一个池"）；
/// ② 跨 hart 归还走的是 `feed`（`home != me` 那条分支）—— 由"hart 1 归还了 hart 0 的块"这件事本身保证；
/// ③ 泵把块**还回了本池**：hart 0 再取时归属仍是自己的池（回路没丢块、没塞错池）。
#[test]
fn pools_are_per_hart_and_cross_hart_frees_route_home() {
    let _a = harness::boot();
    let h = concurrent::cross_hart_handoff(8).expect("跨 hart 归还场景");
    assert_ne!(
        h.pool_a, h.pool_b,
        "两个 hart 取到的块归属同一个池 ⇒ per-hart 池没生效"
    );
    assert_eq!(
        h.returned_home, h.blocks,
        "跨 hart 归还之后，hart 0 取回的块不再归属自己的池（{}/{}）⇒ 泵（feed→suck）没把块转回来",
        h.returned_home, h.blocks
    );
    assert_eq!(concurrent::registry_live(), 0, "场景跑完仍有在册块");
    println!(
        "[mt] 跨 hart 归还：hart0 池 {} / hart1 池 {} / 交回后归属本池 {}/{}",
        h.pool_a, h.pool_b, h.returned_home, h.blocks
    );
}

/// **原子计数器守恒**：并发 churn 一趟，块级账必须**严格**平，帧级/池级只准差池迟滞。
#[test]
fn concurrent_churn_conserves_atomic_counters() {
    let _a = harness::boot();
    let before = concurrent::counters();
    let reports =
        concurrent::concurrent_churn(THREADS, PLAN_BYTES, 0x5EED).expect("并发 churn 无违例");
    let d = concurrent::counters().delta(before);

    // ① 在途块全部归还（原子计数必须归零）。
    assert_eq!(concurrent::registry_live(), 0, "并发跑完仍有在册块");

    // ② 块级原子账**严格守恒**：交付多少块就归还多少块（`block.rs:319/344` 的写侧计数）。
    assert_eq!(
        d.block_take, d.block_give,
        "块级原子账不平（交付 {} / 归还 {}）",
        d.block_take, d.block_give
    );

    // ③ 帧级/池级：差值只能是池的**迟滞**（每池每 size class 至多留 1 页 ⇒ 上界 = 池数 × 档数）。
    assert!(d.frame_take >= d.frame_give, "帧级账反了：{d:?}");
    assert!(
        d.frame_take - d.frame_give <= concurrent::spare_pages_max(),
        "池留下的页数越过迟滞上界（帧级差 {}）：{d:?}",
        d.frame_take - d.frame_give
    );
    assert!(d.pool_take >= d.pool_give, "池级账反了：{d:?}");
    assert!(
        d.pool_take - d.pool_give <= concurrent::spare_pages_max(),
        "池留下的页数越过迟滞上界（池级差 {}）：{d:?}",
        d.pool_take - d.pool_give
    );

    // ④ 真的干了活（不是空跑），且每个线程自身取还配平。
    let taken: usize = reports.iter().map(|r| r.taken).sum();
    assert!(taken > 100, "并发负载太瘦：只交付 {taken} 块");
    for (t, r) in reports.iter().enumerate() {
        assert_eq!(r.taken, r.given, "线程 {t} 取还不平：{r:?}");
    }
    println!(
        "[mt] {THREADS} 线程 × {PLAN_BYTES} B：交付 {taken} 块 / 块级原子账 {}/{} 平 / \
         帧级差 {} / 池级差 {}",
        d.block_take,
        d.block_give,
        d.frame_take - d.frame_give,
        d.pool_take - d.pool_give
    );
}

/// **proptest + 原子计数器**：随机（线程数 × 计划长度）的并发轮次，判据与上一条相同。
///
/// 随机化的是"并发形状"：线程数 1..=4（含单线程退化）、计划长度 8..=96 字节（4..=48 条动作）。
#[test]
fn proptest_concurrent_plans() {
    let _a = harness::boot();
    let mut config = Config::default();
    config.cases = if cfg!(miri) { 2 } else { 24 };
    config.max_shrink_iters = 64;
    // 回归文件关掉：复现方式是把 proptest 打出的最小输入贴回语料（见 README）。
    config.failure_persistence = None;
    let mut runner = TestRunner::new(config);

    runner
        .run(&(1usize..=THREADS, 8usize..=96usize), |(threads, bytes)| {
            let before = concurrent::counters();
            let reports = match concurrent::concurrent_churn(
                threads,
                bytes,
                0xC0FFEE ^ ((threads as u64) << 17) ^ (bytes as u64),
            ) {
                Ok(r) => r,
                Err(v) => {
                    return Err(TestCaseError::fail(format!(
                        "并发 churn 违例：{v}（线程 {threads}，每线程 {bytes} B）"
                    )));
                }
            };
            let d = concurrent::counters().delta(before);
            prop_assert_eq!(concurrent::registry_live(), 0, "并发跑完仍有在册块");
            prop_assert_eq!(
                d.block_take,
                d.block_give,
                "块级原子账不平（交付 {} / 归还 {}）",
                d.block_take,
                d.block_give
            );
            prop_assert!(
                d.frame_take >= d.frame_give
                    && d.frame_take - d.frame_give <= concurrent::spare_pages_max(),
                "帧级差越过迟滞上界：{} - {}",
                d.frame_take,
                d.frame_give
            );
            for (t, r) in reports.iter().enumerate() {
                prop_assert_eq!(r.taken, r.given, "线程 {} 取还不平", t);
            }
            Ok(())
        })
        .unwrap();
    println!("[mt] proptest 并发轮次跑完（判据与守恒用例同一批）");
}

/// **本层自己的对象有没有漏**（`dropcount`）。
///
/// 这条判的不是分配器，而是**读数归因的前提**：在途块的记账对象要是本层自己漏了，
/// LSan / dhat 的读数就不能归给分配器（本 crate 的核心纪律：把"分配器的账"与"对象的
/// 生命周期"分开）。故这里给每一笔取配一个析构计数器，收尾要求**每一笔都恰好析构一次**。
#[test]
fn harness_handles_are_dropped() {
    let _a = harness::boot();
    const PER_THREAD: usize = 24;
    let total = THREADS * PER_THREAD;
    let (counters, viewers) = dropcount::new_vec(total);
    let counters = std::sync::Mutex::new(counters);
    let start = std::sync::Barrier::new(THREADS);
    let layout = Layout::from_size_align(256, 16).expect("块级请求");
    let harts = machine::hart_count().max(1);

    std::thread::scope(|s| {
        for t in 0..THREADS {
            let counters = &counters;
            let start = &start;
            s.spawn(move || {
                machine::hart_bind(t % harts);
                let alloc = block::allocator();
                start.wait();
                let mut held: Vec<(NonNull<u8>, Layout, dropcount::Counter)> =
                    Vec::with_capacity(PER_THREAD);
                for _ in 0..PER_THREAD {
                    let buf = alloc.allocate(layout).expect("块级取用");
                    let p = buf.cast::<u8>();
                    // SAFETY: 本指针本步刚拿到，`layout.size()` 是声明的交付长度。
                    unsafe { core::ptr::write_bytes(p.as_ptr(), 0xA7, layout.size()) };
                    let c = counters
                        .lock()
                        .expect("计数配额锁")
                        .pop()
                        .expect("记账对象配额");
                    held.push((p, layout, c));
                }
                for (p, l, c) in held {
                    for i in 0..l.size() {
                        // SAFETY: 该区间是本线程刚交付的、尚未归还的块。
                        let got = unsafe { *p.as_ptr().add(i) };
                        assert_eq!(got, 0xA7, "交付区被写坏：{:#x}+{i}", p.as_ptr() as usize);
                    }
                    // SAFETY: 同一次 allocate 的产物。
                    unsafe { alloc.deallocate(p, l) };
                    drop(c); // 记账对象随之析构 ⇒ dropcount 计数 +1
                }
            });
        }
    });

    let dropped: usize = viewers.iter().map(|v| v.get()).sum();
    assert_eq!(
        dropped, total,
        "本层有在途记账对象没析构：{dropped}/{total}（读数就不能归给分配器了）"
    );
    assert!(
        counters.lock().expect("计数配额锁").is_empty(),
        "还有没派出去的记账对象"
    );
    assert_eq!(concurrent::registry_live(), 0);
    println!("[mt] dropcount：{total} 个在途记账对象全部恰好析构一次");
}
