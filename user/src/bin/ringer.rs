#![no_std]
#![no_main]

extern crate alloc;

use user::core::mail::MSG_LEN;
use user::core::ring::open;
use user::core::task;
use user::env::{io::put, room::exit};

// ringer：ring 一对一共享内存邮路全链路——Producer push 序号消息，
// 子任务 Consumer pull 校验序号。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
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
    drop(producer);

    exit()
}