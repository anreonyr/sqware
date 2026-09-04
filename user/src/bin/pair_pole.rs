#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;

use ubi::Permission;
use user::core::task;
use user::env::{io::put, mail::PolePie, room};

// pair_pole: 跨 Task 真共享 Pole（页级安全内存 + VEST 派门闩）。
//
// 主任务 = producer：开 Pole、spawn consumer、vest 给 consumer（subset = READ）、
//   map 自己 space（自有 pie 全权 R|W），按 offset 写 16 字节，wake(key)。
// 子任务 = consumer：从 task.pies[0] 拿 vest 来的 Pole、map 自己 space（**R-only**，
//   flags = V|R|U|A|D，cap ⊆ 页表）、按 offset 读 16 字节、校验、wake(key)。
//
// 单轮同步：producer 写完一次 wake，consumer 读完一次 wake；不像 pair 的 N 轮握手。
// Pole 16 字节写不同 offset 互不覆盖，物理页是天然"消息队列"。
//
// 跨模块不变量：consumer 启动时 task.pies = []；内核 vest.rs 把新 pie push 到
// target.pies 末尾 → 落在 [0]。consumer 用 from_idx(0) 拿。

const N: u8 = 16;
const POLE_BYTES: usize = 4096;
const WAIT_MS: usize = 1000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("pair_pole\n");

    let pole = PolePie::open(POLE_BYTES).expect("open");

    // key 钉在堆：跨 task 共享的稳定 usize。
    let key: usize = Box::leak(Box::new([0u8; N as usize])).as_ptr() as usize;

    // spawn consumer。closure 捕获 key（Copy）。
    let join: task::Join<()> = task::closure(move || {
        let pole = PolePie::from_idx(0);
        let va = pole.map().expect("map r-only");
        let ptr = va as *const u8;
        // 等 producer 写完。
        let _ = room::wait(key, WAIT_MS).expect("wait");
        // 读 16 字节不同 offset，校验。
        for i in 0..N {
            let b = unsafe { *ptr.add(i as usize) };
            if b == i {
                let _ = put("K\n");
            }
        }
        let _ = room::wake(key).expect("wake");
    });

    // vest：subset = READ（cap ⊆ 页表 → consumer 拿到的页面 R-only）。
    let _new_idx = pole.vest(join.id(), Permission::READ).expect("vest");

    // map 自己 space：自有 pie 全权 R|W，flags = V|R|W|U|A|D。
    let va = pole.map().expect("map r|w");
    let ptr = va as *mut u8;

    // 写 16 字节到不同 offset。
    for i in 0..N {
        unsafe { *ptr.add(i as usize) = i };
    }
    // 通知 consumer 数据 ready。
    let _ = room::wake(key).expect("wake");

    // 等 consumer 读完。
    let _ = room::wait(key, WAIT_MS).expect("wait");

    // 收尾：先 join（让 consumer 退出），再 shut。
    let _ = join.join();
    let _ = pole.shut();
    let _ = put("pair_pole: done\n");
}
