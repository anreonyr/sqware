#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicUsize, Ordering};

use ubi::Permission;
use user::core::task;
use user::env::{io::put, mail::HolePie, room};

// back: BACK 留位 demo。
//
//   A 是原创建者（vestor=None），开 Hole（R|W|VEST）；
//   A 把 Hole 借给 B（subset=BACK），B.pie.vestor = A；
//   B 测：push 应被拒（无 R）、vest-to-self 应被拒（BACK≠self）、
//   vest-to-vestor 应成功（= A）。
//
// 跨模块不变量：consumer 闭包需知道 A 的 task_id 拿 grantor；A 把自己 id
// 存进 Box::leak 的 AtomicUsize 槽，consumer 读槽拿。

const N: u8 = 8;
const HOLE_MSG_LEN: usize = 64;
const WAIT_MS: usize = 1000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("back\n");

    let my_id = task::self_id().expect("self_id");
    // 单元素 AtomicUsize 槽，存 A 的 task_id。closure 捕获裸指针。
    let slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    slot[0].store(my_id, Ordering::Relaxed);
    let slot_ptr = slot.as_ptr() as usize;

    let key: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    // spawn consumer，捕获 slot_ptr + key。
    let join: task::Join<()> = task::closure(move || {
        // 拿 A 的 task_id（即本 pie 的 vestor）
        let vestor_id = unsafe { (*(slot_ptr as *const AtomicUsize)).load(Ordering::Relaxed) };
        let my_own_id = task::self_id().expect("self_id");

        let hole = HolePie::from_idx(0);
        // 等 A 启动（确保 A 已 vest 完，否则 consumer 的 pie_idx=0 还没创建）
        let _ = room::wait(key, WAIT_MS).expect("wait");

        // 1. push 测试：B 只有 BACK，无 R → 应 Err(Denied)
        let mut msg = [0u8; HOLE_MSG_LEN];
        msg[0] = 0xAA;
        if hole.push(&msg).is_err() {
            let _ = put("F1\n");  // 预期：Denied
        } else {
            let _ = put("B1\n");  // 不应到这
        }

        // 2. vest to self：self ≠ vestor → Err(Denied)
        if hole.vest(my_own_id, Permission::READ).is_err() {
            let _ = put("F2\n");  // 预期
        } else {
            let _ = put("B2\n");
        }

        // 3. vest to vestor (= A)：BACK 守门 target==vestor → Ok
        //    B 只有 BACK，所以 subset ⊆ {BACK}，用 BACK。
        if hole.vest(vestor_id, Permission::BACK).is_ok() {
            let _ = put("V2\n");  // 预期
        } else {
            let _ = put("B3\n");
        }

        let _ = room::wake(key).expect("wake");
    });

    // 主线：开 Hole、写、vest(BACK)、起 consumer、等、收尾
    let hole = HolePie::open().expect("open");
    let mut msg = [0u8; HOLE_MSG_LEN];
    msg[0] = 0x42;
    hole.push(&msg).expect("A push");
    put("P\n");

    hole.vest(join.id(), Permission::BACK).expect("vest back");
    put("V\n");

    let _ = room::wake(key).expect("wake consumer");
    let _ = room::wait(key, WAIT_MS).expect("wait consumer");
    let _ = join.join();
    let _ = hole.shut();
    let _ = put("back: done\n");
}
