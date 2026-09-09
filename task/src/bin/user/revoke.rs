#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::Permission;

use task::core::unit;
use task::env::{
    io::put,
    mail::{HOLE_MTU_MAX, HolePie},
    room,
};

// revoke: 收回授与他人的副本。
//
//   A unseal Hole（全权），accord 给 B（subset = READ|WRITE），拿撤销句柄 token。
//   B 凭 token 重建 Hole，push 成功（R1）。
//   A revoke(B, token) → 副本摘除（V1）。
//   B 再 push → Denied（副本已消失，F1）。
//
// 双 key 协议（防自产自销）：
//   key_grant = A → B  "已 accord + token 存槽"
//   key_done  = B → A  "B 测完一轮"
//
// 跨模块不变量：A 把 token 存进 boxed 槽，B 读槽拿；A 用 B 的 task_id + token 撤销。

const HOLE_MSG_LEN: usize = 64;
// 握手等待 watchdog：协议保证 wake 必到（wait/wake 已闭环防丢唤醒），取 5s 大余量
// 防并发下瞬时延迟误报，同时有限有界——若协议未来又出错，5s 内即可暴露。
const WAIT: usize = 5_000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("revoke\n");

    let hole = HolePie::unseal(HOLE_MTU_MAX).expect("unseal");

    let key_grant: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_done: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let token_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let token_slot_ptr = token_slot.as_ptr() as usize;

    // spawn B。closure 捕获 key + token 槽。
    let join: unit::Join<()> = unit::closure(move || {
        // 等 A accord 完 + 存 token。
        let _ = room::wait(key_grant, WAIT).expect("wait grant");
        let token = unsafe { (*(token_slot_ptr as *const AtomicUsize)).load(Ordering::Relaxed) };
        let hole = HolePie::from_token(token);

        // 1. push 成功：副本带 READ|WRITE。
        let mut msg = [0u8; HOLE_MSG_LEN];
        msg[0] = 0xAA;
        if hole.push(&msg).is_ok() {
            let _ = put("R1\n"); // 预期：副本可用
        } else {
            let _ = put("B1\n");
        }

        // 通知 A "B 已 push 完"，A 此时 revoke。
        let _ = room::wake(key_done).expect("wake after push");

        // 2. revoke 后再 push → Denied（副本已摘除）。
        // 但 B 需要知道 A 已 revoke 完。用独立 key。
        let _ = room::wait(key_grant, WAIT).expect("wait revoked");
        if hole.push(&msg).is_err() {
            let _ = put("F1\n"); // 预期：Denied
        } else {
            let _ = put("B2\n");
        }
        let _ = room::wake(key_done).expect("wake done");
    });

    // accord：subset = READ|WRITE（无 VEST，B 只能 push/pull，不能转授）。
    let token = hole
        .accord(join.id(), Permission::READ | Permission::WRITE)
        .expect("accord");
    token_slot[0].store(token, Ordering::Relaxed);
    put("V\n");

    // 通知 B accord 完。
    let _ = room::wake(key_grant).expect("wake consumer");
    // 等 B push 完（B 会 wake key_done）。
    let _ = room::wait(key_done, WAIT).expect("wait push");

    // revoke：收回授与 B 的副本。
    if hole.revoke(join.id(), token).is_ok() {
        let _ = put("V1\n"); // 预期：撤销成功
    } else {
        let _ = put("B3\n");
    }

    // 通知 B 已 revoke（复用 key_grant）。
    let _ = room::wake(key_grant).expect("wake revoked");
    // 等 B 测完。
    let _ = room::wait(key_done, WAIT).expect("wait done");
    let _ = join.join();
    let _ = hole.seal();
    let _ = put("revoke: done\n");
}
