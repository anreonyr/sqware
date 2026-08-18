#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;

use user::{env, task};

// spawner：验证 task::closure + Join（spawn envcall → U 态 trampoline → 完成槽 → join）。
// 反复派一个算 `0..1000` 的闭包并 join 取回，成功则写 'J'；每轮 sleep 让出免刷屏。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
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
            let _ = env::put(b'J');
        }
        let _ = env::sleep(Duration::from_millis(16));
    }
}
