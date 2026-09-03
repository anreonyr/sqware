#![no_std]
#![no_main]

extern crate alloc;

use user::core::mail::MSG_LEN;
use user::core::ring::open;
use user::core::task;
use user::env::io::put;

// ringer：ring 一对一共享内存邮路全链路——Producer push 序号消息，
// 子任务 Consumer pull 校验序号。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("ringer\n");
    let (producer, consumer) = open(MSG_LEN, 8).expect("ring open");

    let _child = task::closure(move || {
        let mut seen = [0u8; 16];
        for _ in 0..16 {
            let mut buf = [0u8; MSG_LEN];
            consumer.pull(&mut buf).expect("consumer pull failed");
            seen[buf[0] as usize] += 1;
        }
        if seen.iter().all(|&n| n == 1) {
            for _ in 0..16 {
                let _ = put("R\n");
            }
        } else {
            let _ = put("ringer: mismatch!\n");
        }
    });

    for i in 0..16u8 {
        let mut msg = [0u8; MSG_LEN];
        msg[0] = i;
        producer.push(&msg).expect("producer push failed");
    }
    // 关键：等子任务跑完再 drop(producer)——producer 的 Drop 触发
    // ring_close → 移出 task.mail 中的 Ring → Arc<RingMeta> 归零 → unmap。
    // 子任务的 Consumer 是 Copy 数据（不持 kernel Arc），若先 drop 父端
    // Ring，shared 帧在子任务下次 pull 时被释放 → 缺页。
    let _ = _child.join();
    drop(producer);
}
