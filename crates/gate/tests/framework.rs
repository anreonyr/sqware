//! 框架档的门 —— 内核内用例 + 挂起自检（`--features framework`）。搬自 `scripts/framework.sh`（它已删）。
//!
//! # 判据（三条一起）
//!
//!   1) 运行器末行汇总 `[case] cases N ok M fail 0`，且 **N ≥ 1**。**零用例要红**：`.tests` 段
//!      没有引用会被链接器丢掉，症状是**静默零用例**（框架头注点名"比失败更坏"）。
//!   2) 无 `[case] FAIL`、无 `[panic]`。用例失败走 panic 通道（报"哪一例 + 哪一行"再停机），
//!      故失败轮**留不下**汇总行。
//!   3) 停机行 `task: all tasks exited, system halted`。**通过 = 放行启动**（用例跑完接着起
//!      shell），故本门要的是"用例过了**并且**整机照常收尾"。
//!
//! # 照实记（为什么要有这一门）
//!
//! 这一档此前**没有任何门开它**：`framework` 是**内核 crate 的 feature**，不开它，`framework/`
//! 与 `health/` 的用例、以及 `weak` 的出身槽位账与挂起自检**根本不进编译**。于是 hart 号那一刀
//! （`dbaa2cf`）在 `fetch.rs` 的 WFI 钩子上传的是 `HartId`，而钩子那头 `ipi::wfi_entry` /
//! `wfi_exit` 仍收 `usize` —— 两处类型洞，当时**所有门全绿**，直到本门第一次把这一档编出来才响。
//! 教训：**没有门的档 = 没有编译过的档**。
//!
//! # 照实记（档位与喂键）
//!
//! `--profile framework --features framework` 这两个要**一起给**（`Profile::Framework` 一次给
//! 全）；该 profile 继承 `harden` ⇒ `debug_assertions` 开着，故账里的 `debug_assert` 与自检里的
//! 断言**真的在跑**。喂键与 soak 同一教训：**等 `echo: ready` 再喂**——框架档启动更慢（多一批
//! 用例），故上限给到 60 秒。

use gate::*;
use std::time::Duration;

const HALT: &str = "task: all tasks exited, system halted";

#[test]
#[ignore = "要起 QEMU"]
fn framework() {
    let image = build(Scenario::Root, Profile::Framework).expect("造不出那颗要跑的");
    let t = run(&Bench::Machine {
        image,
        env: &[],
        sched: Schedule::OnMark {
            mark: "echo: ready",
            within: Duration::from_secs(60),
            then: vec!["exit".to_string(), "exit".to_string()],
        },
        within: secs(180),
    })
    .expect("机器那一台起不动");
    let _ = t.keep(&log_path("framework"));

    let mut gaps = Vec::new();
    match t
        .text()
        .lines()
        .filter(|l| l.starts_with("[case] cases "))
        .next_back()
    {
        None => gaps.push("无用例汇总行".to_string()),
        Some(s) => {
            let n: usize = s
                .trim_start_matches("[case] cases ")
                .split_whitespace()
                .next()
                .and_then(|x| x.parse().ok())
                .unwrap_or(0);
            if n < 1 {
                gaps.push(format!("零用例（{s}）—— 段被丢了，比失败更坏"));
            }
        }
    }
    if let Some(l) = t.text().lines().find(|l| l.contains("[case] FAIL")) {
        gaps.push(format!("用例失败：{l}"));
    }
    if t.outcome() == Outcome::Panicked {
        gaps.push("内核 panic".to_string());
    }
    if !t.has(HALT) {
        gaps.push("无停机行（用例过了，收尾没到）".to_string());
    }

    assert!(gaps.is_empty(), "框架档没过：\n  {}", gaps.join("\n  "));
    println!(
        "framework: {}",
        t.text()
            .lines()
            .filter(|l| l.starts_with("[case] cases "))
            .next_back()
            .unwrap_or("（无汇总行）")
    );
}
