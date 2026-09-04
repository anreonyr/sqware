#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;

use ubi::Permission;
use user::core::task;
use user::env::{io::put, mail::PolePie, room};

// pair_pole: 跨 Task 真共享 Pole（页级安全内存 + VEST 派门闩 + fault isolation 验证）。
//
// 主任务 = producer：开 Pole、spawn consumer、vest 给 consumer（subset = READ）、
//   map 自己 space（自有 pie 全权 R|W），按 offset 写 16 字节，wake(key_data)，
//   等 consumer 读完 wake(key_done)，shut，打 done。
// 子任务 = consumer：从 task.pies[0] 拿 vest 来的 Pole、map 自己 space（**R-only**，
//   flags = V|R|U|A|D，cap ⊆ 页表）、按 offset 读 16 字节、校验、wake(key_done)、
//   **再写一次 R-only** → StorePageFault → kernel 杀 task（fault isolation）；
//   producer 不 join，靠 wait(key_done) 等 consumer "读完" 的 wake 即可。
//
// 双 key 协议（防自产自销——单 key 下 producer wake 立刻 wait 自己消费 pend）：
//   key_data = producer → consumer  "数据可读"
//   key_done = consumer → producer  "读完通知"
// 不像 pair 的 N 轮握手——Pole 16 字节写不同 offset 互不覆盖，物理页是天然消息队列。
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

    // 两把 key 钉在堆（地址稳定、跨 task 共享）。
    let key_data: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_done: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    // spawn consumer。closure 捕获两把 key（Copy）。consumer 末尾写 R-only 触发
    // fault isolation，被内核杀掉，**不会**走完 closure——producer 不可 join。
    let _join: task::Join<()> = task::closure(move || {
        let pole = PolePie::from_idx(0);
        let va = pole.map().expect("map r-only");
        let ptr = va as *const u8;
        // 等 producer 写完。
        let _ = room::wait(key_data, WAIT_MS).expect("wait data");
        // 读 16 字节不同 offset，校验。
        for i in 0..N {
            let b = unsafe { *ptr.add(i as usize) };
            if b == i {
                let _ = put("K\n");
            }
        }
        // 通知 producer "读完"——producer 拿这个 wake 决定何时 shut。
        let _ = room::wake(key_done).expect("wake done");
        // 测 cap ⊆ 页表：写 R-only 必 StorePageFault；内核走 fault isolation
        // 杀本 task（user 异常隔离，不 panic kernel）。ptr 是 const，强制 mut cast。
        unsafe { *(ptr as *mut u8).add(0) = 0xDE };
    });

    // vest：subset = READ（cap ⊆ 页表 → consumer 拿到的页面 R-only）。
    let _new_idx = pole.vest(_join.id(), Permission::READ).expect("vest");

    // map 自己 space：自有 pie 全权 R|W，flags = V|R|W|U|A|D。
    let va = pole.map().expect("map r|w");
    let ptr = va as *mut u8;

    // 写 16 字节到不同 offset。
    for i in 0..N {
        unsafe { *ptr.add(i as usize) = i };
    }
    // 通知 consumer 数据 ready。
    let _ = room::wake(key_data).expect("wake data");

    // 等 consumer 读完（不是 join：consumer 写 trap 后被清掉，不会自然返回）。
    let _ = room::wait(key_done, WAIT_MS).expect("wait done");

    // 收尾：不 join（consumer 已死）；直接 shut + done。
    let _ = pole.shut();
    let _ = put("pair_pole: done\n");
}
