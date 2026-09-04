#![no_std]
#![no_main]

extern crate alloc;

use user::env::io::put;
use user::env::mail::HolePie;

// porter: Hole 内核邮路单端压力测试——主任务开 Hole、push 后 pull（单槽必须交替）。
// 验证 push/pull 路径 + Permission::READ/WRITE 检查。
// （跨 Task 共享场景见 ringer.rs / docker.rs 的 vest 演示。）

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("porter\n");

    let pie = HolePie::open().expect("hole open");

    // push 10 轮 + 立即 pull（单槽必须交替，否则 slot 满返 Busy）
    let mut ok = true;
    for i in 0..10u8 {
        let mut msg = [0u8; 64];
        msg[0] = i;
        pie.push(&msg).expect("push");

        let mut buf = [0u8; 64];
        pie.pull(&mut buf).expect("pull");
        if buf[0] != i {
            ok = false;
        }
    }
    if ok {
        for _ in 0..10 {
            let _ = put("P\n");
        }
    } else {
        let _ = put("porter: mismatch!\n");
    }
    let _ = put("porter: done\n");
}