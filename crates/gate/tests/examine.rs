//! 验收门 —— **六条判据，缺一不可**（搬自 `scripts/examine.nu`，判据一字未改）：
//!
//!   1) 自行退出：qemu 自己结束（域退场 ⇒ root 收场 ⇒ 级联 ⇒ 停机）⇒ **不是被杀**。
//!   2) 无崩溃：读数里没有 `[panic] at`（内核 panic 报告头）。
//!   3) `echo: ready` —— 域能说一句话，且**不依赖任何服务**（调试面直通固件）。
//!   4) 回显：喂一行进去，同一行回来 —— 能收能回。
//!   5) `task: all tasks exited, system halted` —— 收场那一句（关机的唯一判据）。
//!   6) `router: line=10` —— **中断链在场**：串口驱动开闸 ⇒ 设备拉线 ⇒ PLIC ⇒ 内核摇铃 ⇒
//!      线路由者 claim 到那条线（virt 上 UART0 的中断号是 10）。**回显自己走得上轮询**，
//!      故没有这一条，"中断链断了"与"链子好好的"在日志里长得一模一样。
//!
//! 其余一切（逐步 expect、marker 齐全、实例计数）都是定位手段，不是判据。
//!
//! # 照实记（为什么是定时重复喂，而不是逐步 expect）
//!
//! 逐步 expect 要长驻写端 + 活读日志，是旧门复杂度的大头。调试面读的是固件的串口，**字节先进
//! UART 的 FIFO**（16 字节），guest 什么时候来读都算数，故两句输入都很短（合计 10 字节 < FIFO），
//! 早喂晚喂都不丢。**但"各喂一次"丢过**：实测出现过一整轮"`ping` 已回显、`exit` 没落地 ⇒ 被
//! 超时杀"（1/12）。故**重复喂**：丢一次还有下一次，窗口被盖住——判据全在 guest 侧
//! （回显按**整行相等**、`exit` 谁先读到谁收场），重复不改变任何一条判据的含义。
//!
//! **照实记（`sleep 12` 那条尾巴去哪了）**：旧的喂键器末尾要 `sleep 12`，为的是**别在 guest
//! 收尾之前把写端关掉**。这里不必写那个数——`run` 把写端攥到收尾那一步才放（靠作用域）。

use gate::*;
use std::time::Duration;

const PING: &str = "ping";
const READY: &str = "echo: ready";
const HALT: &str = "task: all tasks exited, system halted";
const IRQ: &str = "router: line=10";

/// 等启动 → **重复**喂 `ping`（盖住慢启动的窗口）→ **重复**喂 `exit`。
fn feed() -> Schedule {
    let mut v = Vec::new();
    for i in 0..6 {
        v.push((Duration::from_secs(4 + i), PING.to_string()));
    }
    for i in 0..4 {
        v.push((Duration::from_secs(10 + i), "exit".to_string()));
    }
    Schedule::Clocked(v)
}

#[test]
fn examine() {
    const REPEAT: usize = 3;

    let image = build(Scenario::Root, Profile::Release).expect("造不出那颗要跑的");
    let out = scene_dir("gate");

    let mut pass = 0;
    let mut why_all = Vec::new();
    for i in 1..=REPEAT {
        let t = run(&Bench::Machine {
            image: image.clone(),
            sched: feed(),
            within: secs(60),
        })
        .expect("机器那一台起不动");
        let _ = t.keep(&out.join(format!("run{i}.log")));

        let echo = t.text().lines().any(|l| l.trim() == PING);
        let mut why = Vec::new();
        if t.outcome() == Outcome::Killed {
            why.push("被超时杀");
        }
        if t.outcome() == Outcome::Panicked {
            why.push("有 panic");
        }
        if !t.has(READY) {
            why.push("缺[echo: ready]");
        }
        if !echo {
            why.push("缺回显[ping]");
        }
        if !t.has(HALT) {
            why.push("缺[task: all tasks exited, system halted]");
        }
        if !t.has(IRQ) {
            why.push("缺中断读数[router: line=10]");
        }

        if why.is_empty() {
            pass += 1;
            println!("run {i}: PASS");
        } else {
            println!("run {i}: FAIL — {}", why.join(" + "));
            why_all.push(format!("run {i}: {}", why.join(" + ")));
        }
    }

    assert_eq!(
        pass,
        REPEAT,
        "验收门 {pass}/{REPEAT}（现场 {}）\n  {}",
        out.display(),
        why_all.join("\n  ")
    );
}
