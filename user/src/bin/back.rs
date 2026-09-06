#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use ubi::Permission;
use user::core::task;
use user::env::{io::put, mail::HolePie, room};

// back: BACK 位 demo。
//
//   A 是原创建者（vestor=None），unseal 两个 Hole（R|W|VEST|BACK）；
//   A 把 hole1 accord 给 B（subset=BACK），B 的副本 vestor = A；
//   A 把 hole2 accord 给 B（subset=VEST|BACK），B 的副本 vestor = A；
//   B 测：
//     纯 BACK：push 被拒（无 R）、accord-to-self 被拒（BACK≠self）、
//       accord-to-vestor 成功（= A）；
//     VEST|BACK：accord-to-self 仍被拒（BACK 压制 VEST 的自由性）、
//       accord-to-vestor 成功（subset=VEST）。
//
// 跨模块不变量：B 需知道 A 的 task_id（即副本 vestor）与两个副本的 token；
// A 把它们存进 boxed 槽，B 读槽拿。
//
// 双 key 协议（防单 key 自产自销）：key_ready = A→B "token 已存槽";
//   key_done = B→A "B 测完一轮"。A 不 join（B 走完即醒 key_done）。

const HOLE_MSG_LEN: usize = 64;
// 握手等待 watchdog：协议保证 wake 必到（wait/wake 已闭环防丢唤醒），取 5s 大余量
// 防并发下瞬时延迟误报，同时有限有界——若协议未来又出错，5s 内即可暴露。
const WAIT: usize = 5_000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("back\n");

    let my_id = task::self_id().expect("self_id");
    // 槽：A 的 task_id + 两个副本 token。
    let vestor_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    vestor_slot[0].store(my_id, Ordering::Relaxed);
    let vestor_slot_ptr = vestor_slot.as_ptr() as usize;
    let tokens_slot: &'static [AtomicU64; 2] = Box::leak(Box::new([const { AtomicU64::new(0) }; 2]));
    let tokens_slot_ptr = tokens_slot.as_ptr() as usize;

    // 双 key 协议（防自产自销）：key_ready = A→B "token 已存槽";
    //   key_done = B→A "B 测完一轮"。
    let key_ready: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_done: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    // spawn consumer，捕获槽指针 + key。
    let join: task::Join<()> = task::closure(move || {
        let vestor_id = unsafe { (*(vestor_slot_ptr as *const AtomicUsize)).load(Ordering::Relaxed) };
        let my_own_id = task::self_id().expect("self_id");
        // 等 A accord 完两个 pie + 存 token。
        let _ = room::wait(key_ready, WAIT).expect("wait ready");

        let tokens = unsafe { &*(tokens_slot_ptr as *const [AtomicU64; 2]) };
        let t1 = tokens[0].load(Ordering::Relaxed);
        let t2 = tokens[1].load(Ordering::Relaxed);
        let hole1 = HolePie::from_token(t1);
        let hole2 = HolePie::from_token(t2);

        // 1. push 测试：B 只有 BACK，无 R → 应 Err(Denied)
        let mut msg = [0u8; HOLE_MSG_LEN];
        msg[0] = 0xAA;
        if hole1.push(&msg).is_err() {
            let _ = put("F1\n");  // 预期：Denied
        } else {
            let _ = put("B1\n");  // 不应到这
        }

        // 2. accord to self：self ≠ vestor → Err(Denied)
        if hole1.accord(my_own_id, Permission::READ).is_err() {
            let _ = put("F2\n");  // 预期
        } else {
            let _ = put("B2\n");
        }

        // 3. accord to vestor (= A)：BACK 守门 dst==vestor → Ok
        //    B 只有 BACK，所以 subset ⊆ {BACK}，用 BACK。
        if hole1.accord(vestor_id, Permission::BACK).is_ok() {
            let _ = put("V2\n");  // 预期
        } else {
            let _ = put("B3\n");
        }

        // 4. VEST|BACK 组合：accord-to-self 仍被拒（BACK 压制 VEST 的自由性）
        if hole2.accord(my_own_id, Permission::VEST).is_err() {
            let _ = put("F3\n");  // 预期：Denied（BACK 守门 dst==vestor，压过 VEST）
        } else {
            let _ = put("B4\n");  // 不应到这：VEST|BACK 不能自由 accord
        }

        // 5. VEST|BACK 组合：accord-to-vestor 成功（subset=VEST ⊆ VEST|BACK）
        if hole2.accord(vestor_id, Permission::VEST).is_ok() {
            let _ = put("V3\n");  // 预期
        } else {
            let _ = put("B5\n");
        }

        let _ = room::wake(key_done).expect("wake done");
    });

    // 主线：unseal 两个 Hole、写、accord(BACK / VEST|BACK)、起 consumer、等、收尾
    let hole1 = HolePie::unseal().expect("unseal hole1");
    let hole2 = HolePie::unseal().expect("unseal hole2");
    let mut msg = [0u8; HOLE_MSG_LEN];
    msg[0] = 0x42;
    hole1.push(&msg).expect("A push");
    put("P\n");

    let t1 = hole1.accord(join.id(), Permission::BACK).expect("accord back");
    let t2 = hole2.accord(join.id(), Permission::VEST | Permission::BACK).expect("accord vest|back");
    tokens_slot[0].store(t1, Ordering::Relaxed);
    tokens_slot[1].store(t2, Ordering::Relaxed);
    put("V\n");

    let _ = room::wake(key_ready).expect("wake consumer");
    let _ = room::wait(key_done, WAIT).expect("wait consumer");
    let _ = join.join();
    let _ = hole1.seal();
    let _ = hole2.seal();
    let _ = put("back: done\n");
}
