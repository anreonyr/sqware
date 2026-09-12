//! 分配器自己的用例 —— **完全不涉及内核对象生命周期**。
//!
//! 判据分四层（与 `lib.rs` 的头注同一张表）：
//!   ① 检测后端（`smartalloc` / `mockalloc` / `dhat` **三选一**，见 README「六条路」）；
//!   ② 本文件 + `harness` 的**影子账**：区间不重叠、指针满足对齐、交付区写读一致、
//!      `block.rs:310` 的请求门必须拒、预算内的合法请求必须成；
//!   ③ 收尾对账：借出去的都还回去之后，影子账与分配器自有簿记都回到起点；
//!   ④ [`crate::fault::allocator_fault`]：分配器**自身**违例的当场炸通道。
//!
//! **本文件的用例不依赖任何检测后端**（四档都跑）：随机序列的确定性语料（LCG）+
//! 伙伴容量守恒 + 耗尽—恢复 + 契约门。proptest 的两条探索腿在 `tests/pairing.rs`
//! 与 `tests/heap.rs`，各自带后端。

use core::alloc::{Allocator, Layout};

use crate::harness::{self, Backend, Op};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;

const PAGE: usize = PAGE_SIZE;

/// 用例收尾：smartalloc 档把孤儿缓冲打出来（其它档 no-op）。
fn done() {
    crate::dump_orphans();
}

/// **接管的凭证（对齐那条契约）**：debug 档的宿主堆由 smartalloc 接管，而它上游只给
/// 8 字节对齐（`sizeof(struct abufhead) == 40`）——违反 Rust `GlobalAlloc` 的契约。
/// 本仓 vendored 的 `smartalloc-sys` 在 C 里把基址抬到 `SM_ALIGN = 64`，于是这里问
/// 一次 64 字节对齐的分配：**拿到的必须真是 64 对齐**。
///
/// 这一条同时钉住两件事：① 全局分配器确实是接管层（系统 malloc 也给 16 对齐，但那条路
/// 由 `--no-default-features` 的反向对照覆盖）；② vendored 补丁真的编进去了。
#[cfg(all(feature = "smartalloc", debug_assertions))]
#[test]
fn host_allocator_took_over() {
    use std::alloc::{Layout, alloc, dealloc};
    let layout = Layout::from_size_align(256, 64).unwrap();
    // SAFETY: layout 非零尺寸；解引用只发生在下面显式写入的范围内。
    unsafe {
        let p = alloc(layout);
        assert!(!p.is_null(), "接管层没给出内存");
        assert_eq!(
            p as usize % 64,
            0,
            "接管层的指针对齐不满足 layout.align()=64（上游 smartalloc 只给 8 字节对齐，\
             见 vendor/smartalloc-sys/csrc/smartall.c 的 SM_ALIGN 段）"
        );
        // 写满整块再还：越界会被 smartalloc 的尾部哨兵当场抓住。
        std::ptr::write_bytes(p, 0xA5, 256);
        dealloc(p, layout);
    }
    done();
}

/// `--no-default-features` / release 档：没有接管层（同一条判据的另一面）。
#[cfg(not(all(feature = "smartalloc", debug_assertions)))]
#[test]
fn host_allocator_not_taken_over() {
    // 未接管时 `dump_orphans()` 是 no-op，这里只做一次普通分配/释放的冒烟。
    let v: Vec<u8> = vec![1u8; 4096];
    assert_eq!(v.len(), 4096);
    drop(v);
    done();
}

/// 帧：单块不同档位取还（1 / 2 / 8 / 32 页），逆序归还逼出伙伴合并。
///
/// 全程由影子账复核：交付区间不重叠、指针页对齐、交付区写读一致。
#[test]
fn frame_alloc_free_round_trip() {
    let _a = harness::boot();
    let ops = [
        Op::Take {
            backend: Backend::Frame,
            req: 0,
        }, // 1 页
        Op::Take {
            backend: Backend::Frame,
            req: 2,
        }, // 2 页
        Op::Take {
            backend: Backend::Frame,
            req: 5,
        }, // 8 页
        Op::Take {
            backend: Backend::Frame,
            req: 7,
        }, // 32 页
        Op::GiveAll, // 逆序归还
    ];
    let r = harness::run(&ops).expect("帧取还往返");
    assert_eq!((r.taken, r.rejected, r.given), (4, 0, 4), "{r:?}");
    // 还尽之后同一串必须能再走一遍（合并把整段拼回去了）。
    let r2 = harness::run(&ops).expect("帧取还往返（第二遍）");
    assert_eq!((r2.taken, r2.given), (4, 4), "{r2:?}");
    done();
}

/// 帧：混合档位churn（64 轮取、水位过 8 就还一块）——影子账抓"同一帧发两次"。
///
/// 序列由代码生成（不是手写），但**确定性**：同一条序列每次跑的都是同一批动作，
/// 失败可复现。随机化的对应物在 `tests/pairing.rs` / `tests/heap.rs`（proptest）。
#[test]
fn frame_no_double_delivery_under_churn() {
    let _a = harness::boot();
    // Miri 下缩到 8 轮：它的成本随**分配器调用次数**线性涨（实测：round-trip 两条秒级，
    // 百次级调用的用例就是分钟级），而 Miri 要找的是 UB 而不是"跑够多少轮"。
    #[cfg(not(miri))]
    let rounds = 64usize;
    #[cfg(miri)]
    let rounds = 8usize;

    let mut ops: Vec<Op> = Vec::new();
    let mut live = 0usize;
    for round in 0..rounds {
        let req = [0u8, 2, 5][round % 3]; // 1 / 2 / 8 页轮转
        ops.push(Op::Take {
            backend: Backend::Frame,
            req,
        });
        live += 1;
        if live > 8 {
            // 还一块在册的（`pick 0` = 影子账数组头；`give` 是 swap_remove ⇒ 它是
            // "某一块在册的"而不是严格最老的那块——这也正是 churn 想要的乱序）。
            ops.push(Op::Give { pick: 0 });
            live -= 1;
        }
    }
    ops.push(Op::GiveAll);
    let r = harness::run(&ops).expect("64 轮混合取还");
    assert!(
        r.taken >= rounds,
        "轮数 {} 对应至少 {} 笔交付：{r:?}",
        rounds,
        rounds
    );
    assert_eq!(r.taken, r.given, "取了多少就该还多少：{r:?}");
    assert!(r.peak_live <= 9, "水位应压在 9 块以内：{r:?}");
    // 中间帧判据（内核那边随 `fence` 一起删了）；宿主侧由影子账兜住：
    // 交付过的帧若被当成块首再放一次，`give` 的模式读回会当场炸。
    assert!(
        r.peak_bytes >= r.peak_live * PAGE,
        "在册峰值字节不该小于在册峰值块数 × 页：{r:?}"
    );
    done();
}

/// 块：混合档位取还 + **契约门**（`block.rs:310` 的越界/超对齐请求必须被拒）。
#[test]
fn block_alloc_free_round_trip() {
    let _a = harness::boot();
    let mut ops: Vec<Op> = [0u8, 3, 5, 7, 9, 10, 12] // 1 / 8 / 24 / 100 / 512 / 1000 / 2000 B
        .into_iter()
        .map(|req| Op::Take {
            backend: Backend::Block,
            req,
        })
        .collect();
    // 越界 / 超对齐：4 条都必须返 Err（不许交付出来）。
    for req in [14u8, 15, 16, 17] {
        ops.push(Op::Take {
            backend: Backend::Block,
            req,
        });
    }
    ops.push(Op::GiveAll);
    let r = harness::run(&ops).expect("块取还 + 契约门");
    assert_eq!(
        r.taken, 7,
        "合法请求必须全部交付（返 Err 只可能是分配器有病）：{r:?}"
    );
    assert_eq!(
        r.rejected, 4,
        "越界/超对齐的 4 条必须被拒（block.rs:310）：{r:?}"
    );
    assert_eq!(r.given, 7, "{r:?}");
    done();
}

/// 伙伴的**容量守恒**：取尽 → 还尽 → 再取尽，页数必须一样；且还尽之后大块必可取回。
///
/// 这是影子账问不了的两件事（影子账只认"我取的那几块"）：
/// * **容量守恒** —— 合并若漏掉某一段，第二轮就取不到同样多的页（帧"丢"了）；
/// * **合并没有漏** —— 还尽之后必须还能拿到 4 MiB 的连续块（整段拼不回来就取不到）；
/// * **`Err` 不是死路** —— 台面满时返 `Err` 而不是 panic（`frame.rs` 的
///   `pull_link` 对污染 `Link` 有意"降级成 `AllocError`"），且它还回一块之后必须立刻可用。
///
/// Miri 下**跳过**：这里要 4000+ 次拆分 + 一次 4 MiB 合并，解释执行下只换来"进度条很慢"，
/// 判据本身（容量守恒）不是 UB；UB 那面由 `frame_alloc_free_round_trip` / 语料 / 并发层覆盖。
#[cfg_attr(
    miri,
    ignore = "解释执行下 4000+ 次 buddy 拆分没有信息增量（UB 由其它用例覆盖）"
)]
#[test]
fn buddy_capacity_is_conserved_and_recovers() {
    let _a = harness::boot();
    let one = Layout::from_size_align(PAGE, PAGE).unwrap();
    let frame = frame::allocator();

    // 第一轮：一块一页，取到 Err 为止。
    let mut held = Vec::new();
    let n1 = harness::fill_frames(1, &mut held);
    assert!(
        n1 * PAGE >= 13 * 1024 * 1024,
        "台面 16 MiB 扣掉 bump 簿记与后备仓仓区（~1 MiB）后应能吐出 13 MiB 以上的单页块，只拿到 {} MiB —— \
         不是真耗尽，是 freelist 提前空了",
        n1 * PAGE / (1024 * 1024)
    );
    // 取尽之后：再问一块必须是 `Err`（不许 panic、不许交付）。
    assert!(
        frame.allocate(one).is_err(),
        "取尽之后仍能交付 ⇒ 容量账不平"
    );

    // `Err` 之后台面仍可用：还回一块，立刻就该能再取到一块。
    let (p, l) = held.pop().expect("第一轮至少拿到一块");
    // SAFETY: `fill_frames` 的产物，layout 原样带回。
    unsafe { frame.deallocate(p, l) };
    let again = frame
        .allocate(one)
        .expect("还回一块之后仍取不到 ⇒ Err 把 freelist 弄坏了");
    // SAFETY: 同一次 allocate 的产物。
    unsafe { frame.deallocate(again.cast::<u8>(), one) };
    harness::drain(&mut held);

    // 第二轮：同样的取法，页数必须一样。
    let mut held2 = Vec::new();
    let n2 = harness::fill_frames(1, &mut held2);
    assert_eq!(n1, n2, "取尽—还尽—再取尽，页数必须一样（合并漏一段就会少）");
    harness::drain(&mut held2);

    // 还尽之后：大块必可取回（伙伴真的拼回来了）。
    let big = Layout::from_size_align(4 * 1024 * 1024, PAGE).unwrap();
    let b = frame
        .allocate(big)
        .expect("还尽之后 4 MiB 的大块必可取回（合并没漏）");
    // SAFETY: 同一次 allocate 的产物。
    unsafe { frame.deallocate(b.cast::<u8>(), big) };
    println!(
        "[capacity] 单页块 {n1} 块（{} MiB）/ 两轮一致 / 4 MiB 大块取回成功",
        n1 * PAGE / (1024 * 1024)
    );
    done();
}

/// 确定性语料：256 条 LCG 计划（每条 96 B ⇒ 48 条动作）走影子账。
///
/// 这条用例是**不装 proptest 也跑**的随机序列版本 —— `plain` 档（零后端、零探索）
/// 同样在压同一批评据。proptest 那两条腿是在它之上再加"探索 + 各自的后端读数"。
///
/// Miri 下**整条跳过**（实测：缩到 6 种子 × 24 B 仍 > 300 s 未完成）。原因是 Miri 的成本
/// 随**分配器调用次数**线性涨（每调用秒级），而这条用例要的是"够多的轮次"—— 那是 proptest
/// 与 LCG 语料在**原生**档的事。Miri 要看的是 **UB**（裸指针来去、交付区写读），由
/// `frame_alloc_free_round_trip` / `block_alloc_free_round_trip` / churn(8 轮) 三条承担。
#[cfg_attr(
    miri,
    ignore = "Miri 下按调用次数计费，语料这条不划算（UB 由三条小用例覆盖）"
)]
#[test]
fn deterministic_corpus() {
    let _a = harness::boot();
    #[cfg(not(miri))]
    let (seeds, bytes_per_plan) = (256u64, 96usize);
    #[cfg(miri)]
    let (seeds, bytes_per_plan) = (6u64, 24usize);

    let (mut ops, mut taken, mut rejected, mut skipped) = (0usize, 0usize, 0usize, 0usize);
    for seed in 0..seeds {
        let bytes = harness::lcg_bytes(seed, bytes_per_plan);
        let plan = harness::plan(&bytes);
        let r = harness::run(&plan).unwrap_or_else(|v| panic!("种子 {seed}：{v}"));
        ops += r.ops;
        taken += r.taken;
        rejected += r.rejected;
        skipped += r.skipped;
    }
    assert!(ops > 20, "语料太瘦：{ops} 条动作");
    assert!(taken > 10, "语料没落到分配器上：只交付 {taken} 块");
    println!(
        "[corpus] {seeds} 种子 × {bytes_per_plan} B：{ops} 条动作 / 交付 {taken} 块 / 拒 {rejected} / 跳过 {skipped}"
    );
    done();
}
