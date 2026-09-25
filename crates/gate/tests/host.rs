//! 宿主档的门 —— **八处纯核心**的规矩，在**宿主**上真跑一遍。
//!
//! 靶场在 `crates/protocol-case`（一个编外 crate、八个测试靶；靶名 = 原来那八个 crate 名去后缀），
//! 判据与那份清单的由来写在它自己的 `Cargo.toml` 头注里，这里不抄第二份。
//!
//! # 判据（四条一起）
//!
//!   1) `cargo test` 退出码 0；
//!   2) **八个靶每个都要有**汇总行 `test result: ok. N passed; 0 failed`，且 **N = 基线**；
//!   3) 全程无 `FAILED`；
//!   4) 总和 **= 118**（多一个靶、少一个靶都拦得住）。
//!
//! **照实记（为什么要有基线）**：这一门原先每台只查"**≥ 1 例**"，于是**少跑**成了唯一看不见的
//! 坏消息——那 34 条 `#[cfg(test)]` 用例只在"某个靶恰好把那几份源码纳进判据"的前提下才跑，
//! 哪个靶一旦不再碰它们，它们会**静默消失**而门照样报绿。基线的口径：**改判据就改这里的数**
//! （那一改会出现在 diff 里，正是一处该被看见的地方）。
//!
//! **照实记（这一门从 `scripts/host.sh` 搬来，判据一字未改；那份脚本已删）**：搬的是外壳——原先那 215 行
//! shell 里，"跑一次 `cargo test` + 逐靶数一遍 + 退出码"这三件里有两件是 `cargo test` 本来就
//! 给的，只有"逐靶基线"是这一门真正要说的那句话。

use gate::*;
use std::collections::VecDeque;

/// 改判据就改这里的数（口径见文件头注）。
const BASELINE: &[(&str, usize)] = &[
    ("operator", 23),
    ("line", 11),
    ("judge", 25),
    ("roster", 20),
    ("board", 11),
    ("quay", 12),
    ("judgement", 10),
    ("supply", 7),
];

const TOTAL: usize = 119;

/// 一个靶的汇总行。
struct Summary {
    name: String,
    state: String,
    passed: usize,
    failed: usize,
}

#[test]
fn host() {
    let bench = Bench::Host {
        manifest: "crates/protocol-case/Cargo.toml",
        within: secs(300),
    };
    let t = run(&bench).expect("宿主那一台起不动");
    let _ = t.keep(&log_path("host"));

    let mut gaps = Vec::new();
    if t.code() != Some(0) {
        gaps.push(format!("cargo test 退出码 {:?}", t.code()));
    }

    let got = summaries(t.text());
    let mut total = 0;
    for (name, want) in BASELINE {
        match got.iter().find(|s| s.name == *name) {
            None => gaps.push(format!("靶 {name} 没有汇总行（它压根没跑？）")),
            Some(s) => {
                if s.state != "ok." {
                    gaps.push(format!("靶 {name} 汇总不是 ok.（{}）", s.state));
                }
                if s.failed != 0 {
                    gaps.push(format!("靶 {name} 有 {} 条失败", s.failed));
                }
                if s.passed != *want {
                    gaps.push(format!(
                        "靶 {name} 用例数 {} ≠ 基线 {want} —— 删了判据就把基线改小并说明理由，加了判据就改大",
                        s.passed
                    ));
                }
                total += s.passed;
            }
        }
    }
    if total != TOTAL {
        gaps.push(format!("总用例数 {total} ≠ {TOTAL}"));
    }
    if let Some(l) = t.text().lines().find(|l| l.contains("FAILED")) {
        gaps.push(format!("有用例失败：{}", l.trim()));
    }

    assert!(gaps.is_empty(), "宿主档没过：\n  {}", gaps.join("\n  "));
    println!("host: {total} 例全过（树 + 线 + 门禁 + 名册/盟籍 + 板 + 会话 + 编排 + 供单）");
}

/// 从 `cargo test` 的读数里收汇总行。
///
/// **归属按启动序（FIFO），不是"最近一条 `Running`"**——照实记（搬这一门时当场撞上的）：
/// `cargo` 打出下一台的 `Running tests/x.rs` 那一刻，上一台的测试进程**还没把最后一块冲出来**
/// （libtest 的 stdio 是块缓冲的），于是读数里长成这样：
///
/// ```text
///      Running tests/judgement.rs (…)     ← 下一台已经报了
/// running 10 tests
/// test … ok
///      Running tests/line.rs (…)         ← 再下一台也报了
/// test the_table_has_a_bottom ... ok     ← judgement 的最后一条
/// test result: ok. 10 passed; …          ← judgement 的汇总，却跟在 line 的 Running 后面
/// ```
///
/// 各台是**依次**跑的，故汇总行的先后 = 启动的先后 ⇒ FIFO 配对是对的，"最近一条"会整体错位一格。
/// 老 `host.sh` 里那条 awk 正是"最近一条"那种写法。
fn summaries(text: &str) -> Vec<Summary> {
    let mut out = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for line in text.lines() {
        let l = line.trim_start();
        if let Some(rest) = l.strip_prefix("Running tests/") {
            queue.push_back(rest.split(".rs").next().unwrap_or("").to_string());
        } else if let Some(rest) = l.strip_prefix("test result: ") {
            let Some(name) = queue.pop_front() else { continue };
            let mut it = rest.split_whitespace();
            let state = it.next().unwrap_or("").to_string();
            let passed = it.next().unwrap_or("0").parse().unwrap_or(0);
            let _ = it.next(); // "passed;"
            let failed = it.next().unwrap_or("0").parse().unwrap_or(0);
            out.push(Summary {
                name,
                state,
                passed,
                failed,
            });
        }
    }
    out
}
