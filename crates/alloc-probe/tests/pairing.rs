//! 第一层·甲：**随机分配/释放序列**的**配对**判据 —— `mockalloc` + `proptest`。
//!
//! # 这一层判什么（以及**不**判什么）
//!
//! `mockalloc` 挂在**全局分配器**上（`lib.rs` 接管点乙），它看得见的是「本 crate 的
//! 每一次宿主堆分配」。被压测的 `frame`/`block` 管的是台面（一块 16 MiB 宿主缓冲）的
//! **内部划分** —— 那不是全局堆，mockalloc 看不见。两者分工是刻意的：
//!
//! | 缺陷 | 谁判 |
//! |---|---|
//! | 分配器**自己的簿记**（记账表 / Vec / Box）漏了、重复释放了、错指针/错尺寸/错对齐归还了 | **本文件**（mockalloc 的配对判词） |
//! | 帧/块**交付**重了、越界了、被自己的记簿踩了、契约门失灵 | `harness` 的影子账（`src/harness.rs`），两条腿共用 |
//!
//! # 实测结论（本层的形状是它决定的，不是设计偏好）
//!
//! `frame.rs` / `block.rs` 的**热路径一次宿主分配都没有**：全部元数据都在 `init` 期
//! 分配好（`frame.rs:165/169/558`、`block.rs:283/296/590`）。所以
//!
//! * 随机序列的每一幕里，mockalloc 的账**恒为空**（判词 `NoData`）—— 这本身就是判据：
//!   「热路径不上全局堆」被固化下来，谁往热路径加一笔宿主分配，这里当场响；
//! * 配对判据的**真身**在**装台幕**：那 9 笔终身表（`harness::INIT_LIFETIME_LEAKS`）
//!   之外的每一笔都必须配对 —— 逐笔对账，多一笔就是有人没还。
//!
//! # 一个必须交代的口径洞
//!
//! mockalloc 的判词（`AllocInfo::finish()`）是**先判笔数**：`num_allocs > num_frees ⇒ Leak`
//! 先返回，于是装台幕里"重复释放 / 错尺寸 / 错对齐"这三类会被 `Leak` **盖住**。
//! 本文件用**双对账**补这个洞：多释放一笔会挪 `num_leaks()`，错尺寸/错对齐会挪
//! `mem_leaked()` —— 两条都对到常量上（见 `host_allocator_init_pairing_ledger`）。
//! 不装 tracing feature（backtrace 那条路要符号化，慢且是另一套判据）。

#![cfg(feature = "mockalloc")]

use alloc_probe::harness;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

/// 计划长度（每条动作 2 字节 ⇒ 最长 64 条动作）。
const PLAN_BYTES: std::ops::RangeInclusive<usize> = 2..=128;
/// proptest 用例数（每条用例一幕；幕内是几十次分配器调用，成本很低）。
const CASES: u32 = 64;

/// **凭证**：mockalloc 真的挂在全局分配器上，且它的账是"笔数"口径。
///
/// 没有这一条，"没检测到缺陷"就分不清是"真干净"还是"检测器没插上" ——
/// 与 `tests.rs::host_allocator_took_over` 用 64 字节对齐当凭证是同一件事。
#[test]
fn mockalloc_is_watching() {
    let info = mockalloc::record_allocs(|| {
        assert!(mockalloc::is_recording(), "幕内 `is_recording()` 应为真");
        let b = Box::new(0x5A5Au64);
        assert_eq!(*b, 0x5A5A);
        drop(b);
    });
    assert!(!mockalloc::is_recording(), "幕外不该还在记录");
    assert_eq!(
        info.num_allocs(),
        1,
        "幕内只有一次分配，却记到 {} 笔",
        info.num_allocs()
    );
    assert_eq!(info.num_frees(), 1);
    assert_eq!(info.mem_allocated(), 8, "u64 那一笔应是 8 字节");
    assert_eq!(info.result(), Ok(()), "一取一还的幕判词必须是 Ok");
}

/// **反向对照**：漏一块必须被点名（`Leak`）。
///
/// 与上一条合起来才成立：检测器会响、且只在真漏时响。
#[test]
fn mockalloc_names_a_deliberate_leak() {
    let info = mockalloc::record_allocs(|| {
        core::mem::forget(Box::new(0u64)); // 故意漏 8 字节
    });
    assert_eq!(info.num_leaks(), 1, "漏的那一笔没被记上：{info:?}");
    assert_eq!(info.result(), Err(mockalloc::AllocError::Leak), "{info:?}");
}

/// **装台幕的逐笔账**：分配器 `init` 期间在宿主堆上做的每一笔分配。
///
/// 判据三条，口径逐条写在 `harness::INIT_*` 上：
/// ① 未配对的**笔数** == 终身表笔数（多一笔 = 有人没还）；
/// ② **分配事件数** == 常量（dhat 那条腿从另一个后端读同一段代码，常数钉在 `harness`）；
/// ③ 分配**字节**与**常驻字节**都落在上界内（"分配器的宿主簿记有多胖"）。
///
/// 收幕判词必然是 `Leak`（终身表按设计不还）；`Leak` 之外的判词（重复释放等）在这个
/// 不平的幕里被盖住 —— 那由 ①③ 的双对账补，见文件头注。
#[test]
fn host_allocator_init_pairing_ledger() {
    let _a = harness::boot(); // 装台幕在哪条用例里开都一样：账存在 harness 里
    let info = harness::init_allocs().expect("装台幕没记账（init 没跑？）");
    let (allocs, frees) = (info.num_allocs(), info.num_frees());
    let (bytes, freed) = (info.mem_allocated(), info.mem_freed());
    println!(
        "[init-ledger] mockalloc：分配 {allocs} 笔 / 释放 {frees} 笔 / 未配对 {} 笔 / \
         分配 {bytes} B / 释放 {freed} B / 未还 {} B",
        info.num_leaks(),
        info.mem_leaked()
    );

    assert_eq!(
        info.num_leaks(),
        harness::INIT_LIFETIME_LEAKS,
        "装台期未配对的**笔数**不对：多出来的那几笔就是没还的分配（分配器的终身表见 \
         harness::INIT_LIFETIME_LEAKS 的逐笔清单）"
    );
    assert_eq!(
        allocs,
        harness::INIT_BLOCKS,
        "装台的**分配事件数**与账不符（dhat 那条腿读同一段代码，常数钉在 harness）"
    );
    assert!(
        bytes <= harness::INIT_BYTES_BUDGET,
        "装台期宿主分配总量 {bytes} B 越过预算 {} B",
        harness::INIT_BYTES_BUDGET
    );
    assert!(
        info.mem_leaked() <= harness::INIT_LIVE_BUDGET,
        "装台期**常驻**（不还的那部分）{} B 越过预算 {} B：分配器的簿记变胖了",
        info.mem_leaked(),
        harness::INIT_LIVE_BUDGET
    );
    assert!(
        matches!(info.result(), Err(mockalloc::AllocError::Leak)),
        "装台幕不平（终身表在那），判词应恰好是 Leak，实得 {:?}",
        info.result()
    );
}

/// **随机分配/释放序列**（proptest）：每条用例一幕，幕内只跑分配器。
///
/// 三层断言：
/// ① 影子账（`harness::run` 的返回值）：区间/对齐/写读/契约门/预算内必成；
/// ② mockalloc 的配对判词：幕内**要么零流量**（`NoData`，热路径不上全局堆的实证），
///    **要么全部配对**（`Ok`）—— 其余五类（Leak/DoubleFree/BadPtr/BadSize/BadAlignment）
///    一律失败；
/// ③ 取还配平：取了多少块就得还多少块（影子账在 `run` 的收尾对账里兜底）。
#[test]
fn random_plans_are_paired() {
    let _a = harness::boot();

    let mut config = Config::default();
    config.cases = CASES;
    config.max_shrink_iters = 256;
    // 关掉回归文件：`Config::default()`（带 `std` 的那份）会挂
    // `FileFailurePersistence::SourceParallel` ⇒ 失败时往 `tests/proptest-regressions/` 写文件。
    // 本 crate 的复现方式是**把 proptest 打出的最小输入贴回语料**（`harness::plan` 的定义
    // 与随机源无关，见 README），不靠那个文件；何况测试不该把仓库写脏。
    config.failure_persistence = None;
    let mut runner = TestRunner::new(config);

    // `TestRunner::run` 收 `Fn`（不是 `FnMut`）⇒ 计数用 `Cell` 记。
    let quiet = std::cell::Cell::new(0usize);
    let busy = std::cell::Cell::new(0usize);
    runner
        .run(
            &proptest::collection::vec(any::<u8>(), PLAN_BYTES),
            |bytes| {
                // 幕外：计划本身（Vec）在这儿分配；幕内只跑 `harness::run`。
                let plan = harness::plan(&bytes);
                let mut out: Option<Result<harness::Report, harness::Violation>> = None;
                let info = mockalloc::record_allocs(|| {
                    out = Some(harness::run(&plan));
                });
                // 幕外：判词、格式化、panic 都不污染上面那一幕的账。
                let report = match out.expect("幕内必跑") {
                    Ok(r) => r,
                    Err(v) => {
                        return Err(TestCaseError::fail(format!(
                            "分配器自身违例：{v}（计划 {bytes:?}）"
                        )));
                    }
                };

                if info.num_allocs() == 0 {
                    prop_assert_eq!(
                        info.result(),
                        Err(mockalloc::AllocError::NoData),
                        "幕内零分配时判词只能是 NoData"
                    );
                    quiet.set(quiet.get() + 1);
                } else {
                    if let Err(e) = info.result() {
                        return Err(TestCaseError::fail(format!(
                            "幕内出现配对错误 {e:?}：分配 {} 笔 / 释放 {} 笔 / \
                             分配 {} B / 释放 {} B（计划 {bytes:?}）",
                            info.num_allocs(),
                            info.num_frees(),
                            info.mem_allocated(),
                            info.mem_freed()
                        )));
                    }
                    busy.set(busy.get() + 1);
                }
                prop_assert_eq!(report.taken, report.given, "取了多少就该还多少");
                Ok(())
            },
        )
        .unwrap();

    println!(
        "[pairing] {CASES} 幕：零宿主流量 {} 幕 / 有流量 {} 幕（后者全配对）",
        quiet.get(),
        busy.get()
    );
    assert_eq!(quiet.get() + busy.get(), CASES as usize);
}
