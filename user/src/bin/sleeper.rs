#![no_std]
#![no_main]
//! sleeper：睡眠 16 毫秒循环（用户面 park），每 64 次唤醒写 'E'（低频心跳防刷屏）。

use core::time::Duration;
use user::env::{put, sleep};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        if n & 0x3F == 0 {
            let _ = put("E\n");
        }
        let _ = sleep(Duration::from_millis(16));
    }
}
