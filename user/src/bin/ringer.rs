#![no_std]
#![no_main]

extern crate alloc;

use user::env::{self, put};
use user::mail::MSG_LEN;
use user::ring::{Consumer, Producer, open};
use user::task;

// ringer：ring 一对一共享内存邮路全链路——主任务建 ring（1 Producer + 1
// Consumer），子任务持 Consumer 并发 pull 校验，主任务 Producer push 序号消息
// （槽满 → wait 阻塞），往返成功打 'R'。验证零拷贝 ring 语义 + 一对一通道
// （无 pier/quay 计数）+ wait/wake 键面（ring 键带独立标记位 RING_KEY_TAG）。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("ringer\n").ok();
    let (producer, consumer) = open(MSG_LEN, 8).expect("ring open");

    // 子任务：Consumer pull 校验序号消息（空 → wait 阻塞）。
    let _child = task::closure(move || {
        let mut seen = [0u8; 16];
        for _ in 0..16 {
            let mut buf = [0u8; MSG_LEN];
            consumer.pull(&mut buf).expect("consumer pull failed");
            seen[buf[0] as usize] += 1;
        }
        // 校验：16 条序号无重无缺。
        if seen.iter().all(|&n| n == 1) {
            for _ in 0..16 {
                put("R\n").ok();
            }
        } else {
            put("ringer: mismatch!\n").ok();
        }
    });

    // 主任务：Producer push 16 轮序号消息（0x00..0x0f）。
    for i in 0..16u8 {
        let mut msg = [0u8; MSG_LEN];
        msg[0] = i;
        producer.push(&msg).expect("producer push failed");
    }
    drop(producer);

    // 通道端固定：主任务退出经 task_exit 钩子清理，子任务 pull 完 16 条自然
    // 结束（不依赖 close 的 Dead 信号）。
    env::exit()
}
