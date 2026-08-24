#![no_std]
#![no_main]
//! sleeper：写 'E' 后睡眠 16 毫秒，循环。

use core::time::Duration;
use user::env::{put, sleep};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    loop {
        let _ = put("E\n");
        let _ = sleep(Duration::from_millis(16));
    }
}
