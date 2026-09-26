// Chrono 域（class 4）—— 读时：tick 计数与单字纳秒。
//
// 本域的本体只住本文件。两格都**不失败**、都**不挂起**，故本域没有词表、也没有落点
// 类型——没有第二种收尾，就没有第二种收尾可表达。

use env::ChronoCall;

use crate::runtime::chrono::{clock, timer};
use crate::runtime::switcher::context::{Gprs, TrapContext};

/// 本域的臂。
pub(super) fn dispatch(frame: &mut TrapContext, call: ChronoCall) {
    match call {
        ChronoCall::Ticks => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        ChronoCall::Clock => {
            // 单字纳秒（自启动基准）：`u128 → u64` 饱和（584 年，实际到不了）。
            let ns = clock::uptime().as_nanos().min(u64::MAX as u128) as u64;
            frame.gpr.set_x(Gprs::A0, ns as usize);
        }
    }
}
