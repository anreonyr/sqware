#![no_std]
#![no_main]

extern crate alloc;

use user::dock::{Pier, open};
use user::env::{self, put};
use user::mail::MSG_LEN;
use user::task;

// docker：dock 共享内存邮路全链路——主任务建 dock（1 pier + 1 quay），子任务
// clone 第二个 pier 并发 push 序号消息（槽满 → wait 阻塞），quay 主任务 pull
// 校验两路序号（槽空 → wait 阻塞），往返成功打 'D'。验证零拷贝 ring 语义 +
// 多 pier 并发 + wait/wake 键面（dock 键带标记位）。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("docker\n").ok();
    let (pier, quay) = open(MSG_LEN, 8).expect("dock open");

    // 第二 pier（clone 计数 +1）：子任务持用——两路并发生产。
    let pier2: Pier = pier.clone();

    // 子任务：pier2 push 8 轮序号消息（0x80+i，与主 pier 的 0x40+i 区分）。
    let _child = task::closure(move || {
        for i in 0..8u8 {
            let mut msg = [0u8; MSG_LEN];
            msg[0] = 0x80 | i;
            pier2.push(&msg).expect("pier2 push failed");
        }
    });

    // 主 pier：push 8 轮序号消息。
    for i in 0..8u8 {
        let mut msg = [0u8; MSG_LEN];
        msg[0] = 0x40 | i;
        pier.push(&msg).expect("pier push failed");
    }

    // quay：拉 16 条校验（两条路各 8 条，序号正确打 'D'）。
    let mut seen = [0u8; 16];
    for _ in 0..16 {
        let mut buf = [0u8; MSG_LEN];
        quay.pull(&mut buf).expect("quay pull failed");
        let tag = if buf[0] & 0x80 != 0 {
            // 子 pier 路：0x80+i → 槽 8..16
            (buf[0] & 0x7f) as usize + 8
        } else {
            (buf[0] & 0x3f) as usize
        };
        seen[tag] += 1;
    }
    // 校验：两路各 8 条、序号无重无缺。
    let good = seen.iter().all(|&n| n == 1);
    if good {
        for _ in 0..16 {
            put("D\n").ok();
        }
    } else {
        put("docker: mismatch!\n").ok();
    }

    drop(pier);
    drop(quay);
    env::exit()
}
