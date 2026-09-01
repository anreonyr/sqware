#![no_std]
#![no_main]

extern crate alloc;

use user::core::dock::{open, Pier};
use user::core::mail::MSG_LEN;
use user::core::task;
use user::env::io::put;

// docker：dock 共享内存邮路全链路——主任务建 dock，子任务并发 push，quay 校验。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("docker\n");
    let (pier, quay) = open(MSG_LEN, 8).expect("dock open");

    let pier2: Pier = pier.clone();

    let _child = task::closure(move || {
        for i in 0..8u8 {
            let mut msg = [0u8; MSG_LEN];
            msg[0] = 0x80 | i;
            pier2.push(&msg).expect("pier2 push failed");
        }
    });

    for i in 0..8u8 {
        let mut msg = [0u8; MSG_LEN];
        msg[0] = 0x40 | i;
        pier.push(&msg).expect("pier push failed");
    }

    let mut seen = [0u8; 16];
    for _ in 0..16 {
        let mut buf = [0u8; MSG_LEN];
        quay.pull(&mut buf).expect("quay pull failed");
        let tag = if buf[0] & 0x80 != 0 {
            (buf[0] & 0x7f) as usize + 8
        } else {
            (buf[0] & 0x3f) as usize
        };
        seen[tag] += 1;
    }
    let good = seen.iter().all(|&n| n == 1);
    if good {
        for _ in 0..16 {
            let _ = put("D\n");
        }
    } else {
        let _ = put("docker: mismatch!\n");
    }

    drop(pier);
    drop(quay);
}