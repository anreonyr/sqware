#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;

use user::core::task;
use user::env::{io::put, room::sleep};

// spawner：反复派一个算 `0..1000` 的闭包并 join 取回。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("spawner\n");
    loop {
        let sum = task::closure(|| {
            let mut s: u64 = 0;
            for i in 0..1000 {
                s = s.wrapping_add(i);
            }
            s
        })
        .join();

        if sum == 499_500 {
            let _ = put("S\n");
        }
        let _ = sleep(Duration::from_millis(1000));
    }
}
