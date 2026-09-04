#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;

use ubi::Permission;
use user::core::task;
use user::env::{io::put, mail::HolePie, room};

// pair: 跨 Task 真共享 Hole。
//
// 主任务 = producer：开 Hole、spawn consumer、vest 给 consumer、push N 条。
// 子任务 = consumer：从 task.pies[0] 拿 vest 来的 Hole、pull N 条。
//
// 双 key 协议（防 push/pull 竞态——单 key 下 consumer wake 后立刻再 pull
// 可能撞上 slot 仍空，producer 还没 push 完）：
//   key_ready = producer → consumer  "数据可读"
//   key_empty = consumer → producer  "槽可写"
//
// producer: while push fails wait(key_empty); push; wake(key_ready)
// consumer: wait(key_ready); pull; wake(key_empty)
//
// 两步握手保证 push 后必有一次配对 wake，wait 与 wake 不会同 key 错位。
//
// 跨模块不变量：consumer 启动时 task.pies = []；内核 vest.rs 把新 pie push 到
// target.pies 末尾 → 落在 [0]。consumer 用 from_idx(0) 拿。

const N: u8 = 16;
const HOLE_MSG_LEN: usize = 64;
const WAIT_MS: usize = 1000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("pair\n");

    let pie = HolePie::open().expect("open");

    // 两把 key 钉在堆（地址稳定、跨 task 共享）。
    let key_ready: usize =
        Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;
    let key_empty: usize =
        Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;

    // spawn consumer。closure 捕获两把 key（Copy）。
    let join: task::Join<()> = task::closure(move || {
        let hole = HolePie::from_idx(0);
        for i in 0..N {
            let _ = room::wait(key_ready, WAIT_MS).expect("wait ready");
            let mut buf = [0u8; HOLE_MSG_LEN];
            hole.pull(&mut buf).expect("pull");
            if buf[0] == i {
                let _ = put("C\n");
            }
            let _ = room::wake(key_empty).expect("wake empty");
        }
    });

    // vest：源 pie 创建时自带 VEST 权；subset = READ ⊆ {R, W, VEST}。
    let _new_idx = pie.vest(join.id(), Permission::READ).expect("vest");

    for i in 0..N {
        let mut msg = [0u8; HOLE_MSG_LEN];
        msg[0] = i;
        while pie.push(&msg).is_err() {
            // slot 满 = consumer 还没 pull；等它 wake(key_empty)。
            let _ = room::wait(key_empty, WAIT_MS).expect("wait empty");
        }
        let _ = room::wake(key_ready).expect("wake ready");
    }

    // 等 consumer 跑完 16 轮再 shut。
    let _ = join.join();
    let _ = pie.shut();
    let _ = put("pair: done\n");
}
