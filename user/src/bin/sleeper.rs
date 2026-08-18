#![no_std]
#![no_main]
//! sleeper：写 'E' 后睡眠 16 毫秒（任务级阻塞：Running → Blocked → unpark 唤醒），
//! 循环。对位旧 blob program_e。

use core::time::Duration;
use user::env::{put, sleep};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    loop {
        let _ = put(b'E');
        let _ = sleep(Duration::from_millis(16));
    }
}
