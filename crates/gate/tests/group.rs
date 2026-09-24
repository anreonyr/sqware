//! 共享组台的门（`SQWARE_ROOT=group`）。搬自 `scripts/group.sh`。
//!
//! # 判据（两条一起）
//!
//!   1) 台主自己判的那一句 `group: PASS`（它把 `group: hung=…` 那个读数算完才说这句话）；
//!   2) 停机行 `task: all tasks exited, system halted`。
//!
//! **照实记（时限 120 秒）**：本台子没有轮次循环，起两台子域 + 200 ms 稳压 + 两次有界等待 ⇒
//! 秒级；卡住就是要红了（那时被期限杀掉，判据自然不过）。
//!
//! **照实记（判 PASS 的是台主，不是这一门）**：这一门只看那一句在不在——把判据做进机器里、
//! 让门只读结论，是本仓一贯的样子（与 `[case]` 协议同源）。

use gate::*;

const HALT: &str = "task: all tasks exited, system halted";

#[test]
#[ignore = "要起 QEMU"]
fn group() {
    let image = build(Scenario::Group, Profile::Debug).expect("造不出那颗要跑的");
    let t = run(&Bench::Machine {
        image,
        env: &[],
        sched: Schedule::None,
        within: secs(120),
    })
    .expect("机器那一台起不动");
    let _ = t.keep(&log_path("group"));

    let verdict = t
        .text()
        .lines()
        .find(|l| l.contains("group: hung="))
        .unwrap_or("无");

    let mut gaps = Vec::new();
    if !t.has("group: PASS") {
        gaps.push(format!("台主未判 PASS（读数：{verdict}）"));
    }
    if !t.has(HALT) {
        gaps.push(format!("无停机行（读数：{verdict}）"));
    }

    assert!(gaps.is_empty(), "共享组台没过：\n  {}", gaps.join("\n  "));
    println!("group: {verdict}");
}
