//! 公平台 —— **持树者公平**那一格的读数台（`SQWARE_ROOT=fair`）。搬自 `scripts/fair.sh`。
//!
//! 场景：`fair` 起的是**同一份引导镜像**，只是编排域那张装配单多一条"聊天客人"（`probe-deep`，
//! 按构建期的 `--cfg sqware_fair` 选），而那一台上它**不让手**（每 16 手才让一次，见
//! `probe_deep.rs` 的粒度表）。受害者是既有的 `echo`——它的那几条读数就是判据
//! （照裁定：**"别人的期限还在一秒内"**）。
//!
//! # 判据（**逐条顺着看，第一个不过就报它**——旧脚本就是 `elif` 那一串）
//!
//!   1. 有停机行（整机不塌）；
//!   2. 聊天客人自己跑完（`probe-deep: tree deep=`）；
//!   3. 它的**五条用例全过**（`[case] probe-deep: 5 cases` / `cases 5 ok 5 fail 0`——判据搬进了
//!      那台探针自己，见 `docs/harness-gate.md` §6）；
//!   4. **受害者的读数没被挤过期**——`echo` 那两条与默认台逐字一致。
//!
//! # 照实记（这一台今天是**红**的，红在哪就是读数）
//!
//! 不让手的条件下，持树者是**串行**的，`echo` 的同步往返会被挤到 1 秒过期（`probe_deep.rs` 头注
//! 那张表：512 层全剪那一档 `soak` 0/10）。修法是调度 / 配额那一族（`docs/fair-gate.md` §4.3），
//! **这一刀只立读数**。故这一门**在修法落地之前会一直红**——那是记录在案的缺口，不是回归。
//!
//! **照实记（喂键等的是 `echo: seq=0`）**：与 soak 等 `member: done` 不同——这一台上要害是
//! `echo`（它就是交互回显），故等它的收尾读数出现再喂 `exit`；上限 90 秒。

use gate::*;
use std::time::Duration;

const HALT: &str = "task: all tasks exited, system halted";

#[test]
#[ignore = "要起 QEMU"]
fn fair() {
    let image = build(Scenario::Fair, Profile::Debug).expect("造不出那颗要跑的");
    let t = run(&Bench::Machine {
        image,
        env: &[],
        sched: Schedule::OnMark {
            mark: "echo: seq=0",
            within: Duration::from_secs(90),
            then: vec!["exit".to_string(), "exit".to_string()],
        },
        within: secs(300),
    })
    .expect("机器那一台起不动");
    let _ = t.keep(&log_path("fair"));

    let echo_tail = || {
        t.text()
            .lines()
            .filter(|l| l.starts_with("echo:"))
            .next_back()
            .unwrap_or("（没有 echo: 那一行）")
            .to_string()
    };

    let why = if !t.has(HALT) {
        let tail = t
            .text()
            .lines()
            .filter(|l| l.contains("fair") || l.contains("probe-deep") || l.starts_with("echo:"))
            .next_back()
            .unwrap_or("（末行不明）");
        Some(format!("无停机行（末行：{tail}）"))
    } else if !t.has("probe-deep: tree deep=") {
        Some("聊天客人没跑完".to_string())
    } else if let Err(g) = count(&t, r"^\[case\] probe-deep: 5 cases[[:space:]]*$", 1) {
        Some(format!("聊天客人的用例没登记全（{g}）"))
    } else if let Err(g) = count(
        &t,
        r"^\[case\] probe-deep: cases 5 ok 5 fail 0[[:space:]]*$",
        1,
    ) {
        Some(format!("聊天客人的用例没过（{g}）"))
    } else if !t.has("echo: list root=0,3") {
        Some(format!("受害者被挤过期：{}", echo_tail()))
    } else if !t.has("echo: list names=sys,device") {
        Some(format!("受害者被挤过期（names）：{}", echo_tail()))
    } else {
        None
    };

    assert!(
        why.is_none(),
        "公平台没过：{}",
        why.unwrap_or_default()
    );
    println!(
        "fair: 受害人读数照旧 · {}",
        t.text()
            .lines()
            .find(|l| l.contains("probe-deep: tree deep="))
            .unwrap_or("")
    );
}
