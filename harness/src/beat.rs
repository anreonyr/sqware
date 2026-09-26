#![no_std]
#![no_main]

//! beat — **到点台的打点者**：量「睡到某个**绝对时刻**」到底漂不漂。
//!
//! # 为什么要有它
//!
//! ABI 里 `RoomCall::Park`（相对毫秒，**下限族**）与 `RoomCall::ParkUntil`（绝对纳秒，
//! 同族）在 2026 年那轮一起定了下来，但 `ParkUntil` **当时没有消费方** ⇒ "到点不漂"
//! 这件事只有签名、没有数字。本程序就是那个消费方：同一台机器上、同一段循环里，
//! **两段各跑 N 次**，把"累计漂移"分别量出来。
//!
//! ```text
//!   A 相对：loop { t0 = clock(); sleep(period); t1 = clock(); drift += (t1-t0) - period; }
//!   B 绝对：next = clock() + period; loop { next += period; sleep_until(next); drift += clock() - next; }
//! ```
//!
//! 判据只有一条：**A 的累计漂移随轮数线性涨**（每轮把"上一轮的迟到"吃进下一轮）；而
//! **B 的迟到不累积**——它每一轮的目标都是绝对时刻，晚到只落在那一轮里。
//!
//! **照实记**：旧注写的是"B 的累计漂移由**最后一轮的迟到**封顶"——那一句对打印出来的
//! `drift_sum_us` **不成立**（两个轴都是 `sum += drift`，逐轮累加）；"不累积"真正读的是
//! **`span_ms` 与 `n × period` 的差**（B 的差 ≈ 初值多出的那一个 `period` + 最后一轮迟到）。
//!
//! # 怎么跑它
//!
//! ```text
//!   cargo image beat && QEMU_ICOUNT= cargo run --release    # 与验收门同环境（必须）
//!                       QEMU_ICOUNT= cargo run --release    # 对照：icount 开（唤醒被节流）
//! ```
//!
//! 两档都值得跑：icount 开时"一记唤醒"是毫秒级（见 `scripts/boot.nu` 的注释与 rig 的
//! 照实记），A 会漂得更凶；B 不该被它带跑——**这正是"绝对到点"买下的东西**。
//!
//! # 读数
//!
//! 每档一行：`beat: rel n=… period_ms=… drift_sum_us=… drift_max_us=… drift_min_us=… span_ms=…`
//! `span_ms` = 第一轮到最后一轮的**真实跨度**（与 `n × period` 比：差得越多越漂）。

extern crate alloc;
extern crate programs;

use core::time::Duration;

use runtime::env::chrono;
use runtime::env::room;
use protocol::debug;

/// 每轮要的周期（毫秒）。
const PERIOD_MS: u64 = 5;
/// 每档跑多少轮。
const N: usize = 200;

/// `()` = "没有失败要报"（`Exit for ()` ⇒ `EXIT_OK`）——本台子跑完就是结论。
#[programs::entry]
fn main() {
    let period_ns = PERIOD_MS * 1_000_000;

    // ── A 相对：每轮"至少睡 period" ⇒ 上一轮的迟到被下一轮吃进累计漂移 ──
    let start = now_ns();
    let mut sum: i64 = 0;
    let mut max: i64 = i64::MIN;
    let mut min: i64 = i64::MAX;
    for _ in 0..N {
        let t0 = now_ns();
        let _ = room::sleep(Duration::from_millis(PERIOD_MS));
        let t1 = now_ns();
        let drift = t1.saturating_sub(t0) as i64 - period_ns as i64;
        sum += drift;
        max = max.max(drift);
        min = min.min(drift);
    }
    let span_rel = now_ns().saturating_sub(start);
    debug!(
        "beat: rel n={N} period_ms={PERIOD_MS} drift_sum_us={} drift_max_us={} drift_min_us={} span_ms={}",
        sum / 1000,
        max / 1000,
        min / 1000,
        span_rel / 1_000_000
    );

    // ── B 绝对：到点是绝对的 ⇒ 迟到**不落进下一轮**（`drift_sum_us` 仍是逐轮累加，
    //    真正体现"不累积"的是 `span_ms`：它 ≈ n × period + 初值那一个 period + 末轮迟到）──
    let start = now_ns();
    let mut sum: i64 = 0;
    let mut max: i64 = i64::MIN;
    let mut min: i64 = i64::MAX;
    let mut next = now_ns() + period_ns;
    for _ in 0..N {
        next += period_ns;
        let _ = room::sleep_until(next);
        let drift = now_ns() as i64 - next as i64;
        sum += drift;
        max = max.max(drift);
        min = min.min(drift);
    }
    let span_abs = now_ns().saturating_sub(start);
    debug!(
        "beat: abs n={N} period_ms={PERIOD_MS} drift_sum_us={} drift_max_us={} drift_min_us={} span_ms={}",
        sum / 1000,
        max / 1000,
        min / 1000,
        span_abs / 1_000_000
    );

    // 差值就是本台子要说的那句话：同一台机器上，A 的 span 与 B 的 span 差多少。
    debug!(
        "beat: total rel_span_ms={} abs_span_ms={} diff_ms={}",
        span_rel / 1_000_000,
        span_abs / 1_000_000,
        (span_rel as i64 - span_abs as i64) / 1_000_000
    );
    // 跑完 = 报 `EXIT_OK`（`()` 折出来的那个码），不必再写一遍。
}

/// 自启动基准的纳秒标量（与 `sleep_until` 的 `at` 同基准同单位）。
fn now_ns() -> u64 {
    chrono::clock()
}

