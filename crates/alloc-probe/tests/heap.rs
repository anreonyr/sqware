//! 第一层·乙：**随机工作负载**下的宿主堆**用量**判据 —— `dhat` + `proptest`。
//!
//! # 这一层判什么
//!
//! dhat 挂在**全局分配器**上（`lib.rs` 接管点丙），给出三套读数：**累计**（`total_*`）、
//! **当下**（`curr_*`）、**峰值**（`max_*`）的块数与字节数。于是"分配器自己占了多少宿主
//! 内存"第一次有了能断言的气压计，判据三条：
//!
//! | 判据 | 断言 | 违反意味着 |
//! |---|---|---|
//! | 凭证：尺子准 | 一次已知分配 ⇒ `Δtotal_blocks == 1` 且 `Δtotal_bytes == 4096` | dhat 没插上，或口径不是字节 |
//! | 装台预算 | init 的 `Δtotal_blocks` 等于常数、`Δtotal_bytes`/`Δcurr_bytes` 落在预算内 | 分配器的簿记变胖，或有人加了一笔没还的分配 |
//! | 稳态零漂移 | 每一幕 `Δtotal_bytes == Δcurr_bytes == Δmax_bytes == 0` | 热路径偷偷上全局堆，或每轮都在长常驻内存 |
//!
//! # 为什么是**一个** `#[test]`
//!
//! dhat 的 `Profiler` 一个进程只能建一个（上游 `build()` 里断言旧的是 `None`），而且
//! 读数是**进程级**的 —— 同进程里别的用例的分配会算进我们的增量。集成测试文件
//! （`tests/heap.rs`）本身就是**独立进程**，故这里只放一条用例，三段判据在它里顺序走。
//! 这也是 `.cargo/config.toml` 钉 `RUST_TEST_THREADS = 1` 的第二个理由。
//!
//! # 与 mockalloc 那条腿的关系
//!
//! 两个后端**从不同口径读同一段代码**（装台期的那些分配），故它们的账按定义必须相等：
//! `num_allocs ↔ Δtotal_blocks`、`mem_allocated ↔ Δtotal_bytes`、`mem_leaked ↔ Δcurr_bytes`。
//! 常数因此钉在 `harness::INIT_*` 一处，谁漂了另一条腿也会响。

#![cfg(feature = "dhat")]

use alloc_probe::harness;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

/// 凭证用的已知分配：`vec![x; N]` 走**一次** `alloc` + memset。
const CANARY_BYTES: u64 = 4096;
/// 计划长度（每条动作 2 字节 ⇒ 最长 64 条动作）。
const PLAN_BYTES: std::ops::RangeInclusive<usize> = 2..=128;
/// proptest 用例数（每条用例一幕）。
const CASES: u32 = 64;
/// 整幕跑完时给 **proptest 自己**留的余量：它的 RNG/策略树/计数是进程里常驻的 KB 级
/// 对象，不属于分配器的账。分配器的账是**幕内**逐幕判的（下面三条 `prop_assert_eq!`），
/// 这一条只兜"整幕跑完不留大块常驻"。
const RUNNER_SLACK: usize = 64 * 1024;

#[test]
fn heap_usage_under_random_workload() {
    // testing 档：不写 `dhat-heap.json`，只供断言（上游文档：heap usage testing）。
    let _profiler = dhat::Profiler::builder().testing().build();

    // ── ① 凭证：尺子插上了，且口径是字节 ────────────────────────────────────
    // "没读出问题"与"没插上尺子"必须分得开 —— 与 smartalloc 那条腿拿 64 字节对齐
    // 当凭证、mockalloc 那条腿拿"一取一还 = 1 笔"当凭证，是同一件事。
    let c0 = dhat::HeapStats::get();
    {
        let v: Vec<u8> = vec![0x5A; CANARY_BYTES as usize];
        core::hint::black_box(&v);
    }
    let c1 = dhat::HeapStats::get();
    dhat::assert_eq!(
        c1.total_blocks - c0.total_blocks,
        1,
        "dhat 没记到那一次分配（全局分配器不是 dhat::Alloc？）"
    );
    dhat::assert_eq!(
        c1.total_bytes - c0.total_bytes,
        CANARY_BYTES,
        "dhat 的字节口径对不上"
    );
    dhat::assert_eq!(
        c1.curr_bytes,
        c0.curr_bytes,
        "释放之后 curr_bytes 没回到原处"
    );

    // ── ② 装台预算：分配器 init 在宿主堆上花了多少、留下多少 ────────────────
    // 台面（16 MiB 宿主缓冲）是**测试自己的**，先进来 —— 否则它会盖掉分配器那几 KB 的账。
    let arena = harness::arena();
    assert!(arena.pages() >= 4096, "台面应有 16 MiB");

    let b0 = dhat::HeapStats::get();
    harness::init(); // 装台：configure + bump → block → frame（幕 = 这两行之间）
    let b1 = dhat::HeapStats::get();
    let blocks = b1.total_blocks - b0.total_blocks;
    let bytes = b1.total_bytes - b0.total_bytes;
    let live = b1.curr_bytes as i64 - b0.curr_bytes as i64;
    println!("[init-ledger] dhat：分配 {blocks} 笔 / 分配 {bytes} B / 常驻 {live} B");

    assert!(live >= 0, "装台期常驻字节不可能是负的：{live}");
    dhat::assert_eq!(
        blocks,
        harness::INIT_BLOCKS,
        "装台的分配事件数与账不符（mockalloc 那条腿读同一段代码，常数钉在 harness）"
    );
    dhat::assert!(
        bytes <= harness::INIT_BYTES_BUDGET,
        "装台期宿主分配总量 {bytes} B 越过预算 {} B",
        harness::INIT_BYTES_BUDGET
    );
    dhat::assert!(
        live as u64 <= harness::INIT_LIVE_BUDGET,
        "装台期**常驻** {live} B 越过预算 {} B：分配器的簿记变胖了",
        harness::INIT_LIVE_BUDGET
    );
    dhat::assert!(live > 0, "装台没留下任何常驻字节 ⇒ 终身表没建起来？");

    // 装台**幂等**：再调一次不该再分配（`OnceLock` 的语义，也是"终身表只建一次"的判据）。
    let i0 = dhat::HeapStats::get();
    harness::init();
    let i1 = dhat::HeapStats::get();
    dhat::assert_eq!(
        i1.total_bytes,
        i0.total_bytes,
        "装台不幂等：第二次 init 又分配了"
    );
    dhat::assert_eq!(i1.curr_bytes, i0.curr_bytes);

    // ── ③ 随机工作负载：幕内零增量、且不抬高峰值 ────────────────────────────
    let baseline = dhat::HeapStats::get();
    let mut config = Config::default();
    config.cases = CASES;
    config.max_shrink_iters = 128;
    // 关掉回归文件：`Config::default()`（带 `std` 的那份）会挂
    // `FileFailurePersistence::SourceParallel` ⇒ 失败时往 `tests/proptest-regressions/` 写文件。
    // 本 crate 的复现方式是**把 proptest 打出的最小输入贴回语料**（`harness::plan` 的定义
    // 与随机源无关，见 README），不靠那个文件；何况测试不该把仓库写脏。
    config.failure_persistence = None;
    runner_workload(&mut TestRunner::new(config), baseline);
}

/// 随机工作负载那一段（拆出来只为让上面那条用例读起来是"凭证 → 预算 → 负载"三步）。
fn runner_workload(runner: &mut TestRunner, baseline: dhat::HeapStats) {
    // `TestRunner::run` 收 `Fn`（不是 `FnMut`）⇒ 计数用 `Cell` 记。
    let quiet = std::cell::Cell::new(0usize);
    runner
        .run(
            &proptest::collection::vec(any::<u8>(), PLAN_BYTES),
            |bytes| {
                // 幕外：计划本身（Vec）在这儿分配，幕内只跑 `harness::run`。
                let plan = harness::plan(&bytes);
                let w0 = dhat::HeapStats::get();
                let out = harness::run(&plan); // 幕内：分配器热路径
                let w1 = dhat::HeapStats::get();
                let d_total = w1.total_bytes - w0.total_bytes;
                let d_curr = w1.curr_bytes as i64 - w0.curr_bytes as i64;
                let d_peak = w1.max_bytes as i64 - w0.max_bytes as i64;
                // 幕后才释放：幕内释放会把别人的块算进这一幕的账。
                drop(plan);

                if let Err(v) = out {
                    return Err(TestCaseError::fail(format!(
                        "分配器自身违例：{v}（计划 {bytes:?}）"
                    )));
                }
                // 三条零增量：热路径一次都不上全局堆（`frame.rs`/`block.rs` 的元数据
                // 全在装台期分配好），故累计不涨、常驻不涨、峰值也抬不起来。
                // 这是**实测发现**的固化：哪天真要在热路径上加宿主分配，先让它配对，
                // 再把这三条改成上界断言，并把发现写进 README。
                prop_assert_eq!(d_total, 0, "工作负载在宿主堆上分配了 {} 字节", d_total);
                prop_assert_eq!(d_curr, 0, "幕结束时常驻字节变了 {} 字节", d_curr);
                prop_assert_eq!(d_peak, 0, "工作负载把宿主堆峰值抬高了 {} 字节", d_peak);
                quiet.set(quiet.get() + 1);
                Ok(())
            },
        )
        .unwrap();

    let end = dhat::HeapStats::get();
    dhat::assert!(
        end.curr_bytes <= baseline.curr_bytes + RUNNER_SLACK,
        "整幕跑完常驻字节涨了 {} B（分配器不留常驻宿主内存；余量只给 proptest 自己）",
        end.curr_bytes as i64 - baseline.curr_bytes as i64
    );
    println!(
        "[heap] {CASES} 幕：幕内 Δtotal_bytes / Δcurr_bytes / Δmax_bytes 全 0（{} 幕）",
        quiet.get()
    );
}
