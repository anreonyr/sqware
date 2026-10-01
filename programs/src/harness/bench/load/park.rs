#![no_std]
#![no_main]

//! park — 忙机台（`load`）的**打点者**：只做一件事——`Park{1ms}` 无限循环。
//!
//! # 为什么不用 `churn` 当打点者
//!
//! `churn` 一上来要 `tick::calibrate()`（睡 200 ms + 忙等两格刻度），而在**满负荷**的机器上
//! 那段校准要耗掉好几个量子——实测：台主空转跑完了，4 枚 `churn` 还卡在自校准里，整场只有
//! 1 枚到点到期（`late_n=1`）。打点者不该有启动成本，也不该自己占核：占核是 `busy` 的活。
//!
//! 于是本程序=**唯一被测的那条调用**：每睡 1 ms 就登记一枚到点；`load` 那几枚 `busy` 负责让
//! 每一颗核一刻不闲（任一颗空闲核都会替全局兑现到点，那正是这条债的隐藏者）。

extern crate programs;

use core::time::Duration;

use runtime::env::room;

/// 每轮睡多久（毫秒）——就是被测的那个 `millis`。
const MS: u64 = 1;

#[programs::entry]
fn main() -> ! {
    loop {
        let _ = room::sleep(Duration::from_millis(MS));
    }
}
