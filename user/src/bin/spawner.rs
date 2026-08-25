#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;

use user::{
    env::{self, put},
    task,
};

// spawner：验证 task::closure + Join：反复派一个算 `0..1000` 的闭包并 join 取回，成功则写 'J'。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("spawner\n").ok();
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
            env::put("S\n").ok();
        }
        env::sleep(Duration::from_millis(1000)).ok();
    }
}
