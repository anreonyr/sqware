//! 忙机台的门（**景 `load`**）。搬自 `scripts/load.sh`（它已删）。
//!
//! # 判据（三条一起）
//!
//!   1) `load: spawned rows=…`（负荷真铺开了）；
//!   2) 停机行 `task: all tasks exited, system halted`（自己收干净，不是被期限杀掉）；
//!   3) 紧随其后的内核读数 `timer: late_n=…`。
//!
//! # 照实记（核数就是那条债的开关）
//!
//! 这条债要"核一刻不闲"才成立——`redeem` / `drain` 是**全局**的，**任一颗空闲核**都会按 `due()`
//! 武装、在到点那一刻替全局兑现，故 `soak`（全员 1 ms 轮询睡眠）与 `rig`（只有一枚 `hang`）都
//! 量不到它。本台子用占核者（`busy`）把那颗核钉住，再用一枚稀疏打点者（`park`）把"到点兑现迟到"
//! 量出来。故 **`QEMU_SMP=1` 是"机制隔离档"**，`=4` 是**对照档**（有核空闲 ⇒ 债被盖住，这正是
//! 树内量不出来的原因）。实测（icount 关、release、n=81）：修复前 `late_avg=97 / late_max=99 ms`；
//! 修复后 `0 / 0 ms`（`late_max_tick=4800` ≈ 480 µs），`traps` 两边一致（643）。
//!
//! 那一格因此走 `Bench::Machine` 的 `env`（透给 `boot.nu` 的旋钮），不写死在场景里——同一张
//! 装配单要能跑两档。
//!
//! **照实记（没有喂键器）**：负荷铺开、量完、自己收场；日程是明写的 `Schedule::None`。

use gate::*;

const HALT: &str = "task: all tasks exited, system halted";

#[test]
#[ignore = "要起 QEMU"]
fn load() {
    let image = build(Scenario::Load, Profile::Debug).expect("造不出那颗要跑的");
    let t = run(&Bench::Machine {
        image,
        env: &[("QEMU_SMP", "1")],
        sched: Schedule::None,
        within: secs(120),
    })
    .expect("机器那一台起不动");
    let _ = t.keep(&log_path("load"));

    let mut gaps = Vec::new();
    if !t.has("load: spawned rows=") {
        let tail = t
            .text()
            .lines()
            .filter(|l| l.contains("load:"))
            .next_back()
            .unwrap_or("（没有 load: 那一行）");
        gaps.push(format!("负荷没铺开（末行：{tail}）"));
    }
    if !t.has(HALT) {
        gaps.push("无停机行".to_string());
    }
    if !t.has("timer: late_n=") {
        gaps.push("无内核读数（timer: late_n=）".to_string());
    }

    assert!(gaps.is_empty(), "忙机台没过：\n  {}", gaps.join("\n  "));
    println!(
        "load: {}",
        t.text()
            .lines()
            .find(|l| l.contains("timer: late_n="))
            .unwrap_or("")
    );
}
