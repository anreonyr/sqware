#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;

use ubi::ucall;
use user::{
    env::{self, put},
    task,
};

// spawner：验证 task::closure + Join（spawn envcall → U 态 trampoline → 完成槽 → join）。
// 反复派一个算 `0..1000` 的闭包并 join 取回，成功则写 'J'；每轮 sleep 让出免刷屏。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("FUCK\n").ok();
    // 用户主动 panic 方式（显式 envcall）：a7=Panic(9)，a0=关联码；内核 panic!
    // 并转储场景（呼叫人即 running 任务 → ubt/CSR 符号化完整）。等效封装见
    // user::env::panic_me(code)。此处用原始构建器直拼（镜像 ubi ABI）。
    let _ = ucall::UcallBuilder::new(ubi::Ucall::Panic)
        .args(ucall::UArgs {
            a0: 0xDEAD,
            ..Default::default()
        })
        .call();
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
            let _ = env::put("J");
        }
        let _ = env::sleep(Duration::from_millis(16));
    }
}
