#![no_std]
#![no_main]

extern crate alloc;

use user::env::io::put;
use user::env::mail::PolePie;

// ringer: Pole 共享内存邮路单端压力测试——主任务开 Pole、map 自己 space，
// 写 16 字节到不同偏移、读回校验。
// （跨 Task 共享场景见 docker.rs 多 pier 演示。）

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("ringer\n");

    let pole = PolePie::open(4096).expect("pole open");
    let va = pole.map().expect("map");
    let ptr = va as *mut u8;

    // 写 16 字节到不同偏移（[0..16)）
    for i in 0..16u8 {
        unsafe { *ptr.add(i as usize) = i; }
    }
    // 校验
    let mut ok = true;
    for i in 0..16u8 {
        let b = unsafe { *ptr.add(i as usize) };
        if b != i {
            ok = false;
        }
    }
    if ok {
        for _ in 0..16 {
            let _ = put("R\n");
        }
    } else {
        let _ = put("ringer: mismatch!\n");
    }
    let _ = put("ringer: done\n");
}