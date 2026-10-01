#![no_std]
#![no_main]

//! churn — 压测台的**受害者**：不停地在"挂着"与"在台上"之间换。
//!
//! ```text
//!   loop { 空转 BURST_MS 毫秒（在台上，纯用户态，不落核）; sleep(BURST_MS)（离核） }
//! ```
//!
//! # 为什么是这个形状
//!
//! 「他杀偶发不生效」那一格要的状态是：**点名的那一刻它正走在"离核"的路上**——那样那一记
//! 定向 IPI 就会打在别的上下文上，而它已经挂起、再也不是"本核当前任务"。故受害者要
//! **频繁地**在两种状态之间换，且"在台上"那一段要**短到与投递延迟同量级**。
//!
//! "在台上"那一段的**实际轮数**由启动时自校准得到（[`tick::calibrate`]）：毫秒基准取自
//! 内核的 `sleep`，故换机器/换 CPU 不用改常量——台主那边扫的也是同一把尺。
//!
//! 本程序不铸孔、不要门闩、不建会话：`sleep` 一个调用就够（`Park` 是内核既有的那条路）。
//!
//! 今天没有自动跑它的入口（验收门迁到 `embedded-test` 后，那两个
//! 手动入口（`crates/gate/tests/console.rs` / `scripts/probe.sh`）都已删）。
//! 本程序是 **`hang` 的前身**——`rig` 现在跑 `hang` 而不是反复造/杀 `churn` 来扫"那一刻"
//! （见 `rig.rs` 头注）。本程序留在清单里备查。
//!
//! **特权级由清单定**：本域是 U 态（`programs::unit::PROGRAMS` 里这一行的 `kind`）——压测要的是挂起/在台
//! 的切换，与特权级无关，故给最小特权那一档。

extern crate programs;

use programs::harness::tick;

use core::time::Duration;

use runtime::env::room;

#[programs::entry]
fn main() -> ! {
    // 自校准：本机"在台上"那一段 = 多少轮空转（与台主扫时序用的是同一把尺）。
    let (iters_per_ms, _ms_per_tick) = tick::calibrate();
    let burst = iters_per_ms.saturating_mul(tick::BURST_MS);
    loop {
        tick::spin(burst); // 在台上
        let _ = room::sleep(Duration::from_millis(tick::BURST_MS as u64)); // 离核
    }
}
