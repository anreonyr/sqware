//! 压测台的门（**景 `rig`**）。搬自 `scripts/stress.sh`（它已删）。
//!
//! # 判据（两条一起）
//!
//!   1) 汇总行 `rig: total`（那一台自己把总数报出来了）；
//!   2) 停机行 `task: all tasks exited, system halted`。
//!
//! **照实记（没有喂键器）**：这台子不需要外来的 `exit`——它自己"造 → 握手（认下它交回来的孔）
//! → 唤醒 → 杀 → 判"，判完就收场。故日程是 `Schedule::None`（**不是**"忘了喂"：`None` 是明写
//! 的一格）。
//!
//! **照实记（时限 300 秒）**：rig A 之后每一轮都要走上面那一串，而判决窗口是 300 ms + 1 秒
//! 宽限 ⇒ 命中 `late` / `lost` 的那些轮本来就慢。旧脚本把外接 `timeout` 放宽到 300 就是这件事。

use gate::*;

const HALT: &str = "task: all tasks exited, system halted";

#[test]
#[ignore = "要起 QEMU"]
fn stress() {
    let image = build(Scenario::Rig, Profile::Release).expect("造不出那颗要跑的");
    let t = run(&Bench::Machine {
        image,
        env: &[],
        sched: Schedule::None,
        within: secs(300),
    })
    .expect("机器那一台起不动");
    let _ = t.keep(&log_path("stress"));

    let mut gaps = Vec::new();
    if !t.has("rig: total") {
        let tail = t
            .text()
            .lines()
            .filter(|l| l.contains("rig:"))
            .next_back()
            .unwrap_or("（没有 rig: 那一行）");
        gaps.push(format!("无汇总行（末行：{tail}）"));
    }
    if !t.has(HALT) {
        gaps.push("无停机行".to_string());
    }

    assert!(gaps.is_empty(), "压测台没过：\n  {}", gaps.join("\n  "));
    println!(
        "stress: {}",
        t.text()
            .lines()
            .find(|l| l.contains("rig: total"))
            .unwrap_or("")
    );
}
