#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};

use ubi::Permission;
use user::core::task;
use user::env::{io::put, mail::HolePie, room};

// pair: 跨 Task 真共享 Hole。
//
// 主任务 = producer：unseal Hole、spawn consumer、accord 给 consumer、push N 条。
// 子任务 = consumer：凭 accord 返回的 token 重建 Hole、pull N 条。
//
// 三 key 协议（token 传递与消息循环分离，避免单 key 自产自销）：
//   key_token = producer → consumer  "token 已存槽"
//   key_ready = producer → consumer  "数据可读"
//   key_empty = consumer → producer  "槽可写"
//
// producer: accord → store token → wake(key_token) → [while push fails wait(key_empty);
//           push; wake(key_ready)] × N
// consumer: wait(key_token) → read token → [wait(key_ready); pull; wake(key_empty)] × N

const N: u8 = 16;
const HOLE_MSG_LEN: usize = 64;
const WAIT_MS: usize = 1000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("pair\n");

    let pie = HolePie::unseal().expect("unseal");

    // 三把 key 钉在堆（地址稳定、跨 task 共享）。
    let key_token: usize = Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;
    let key_ready: usize = Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;
    let key_empty: usize = Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;

    let token_slot: &'static [AtomicU64; 1] = Box::leak(Box::new([AtomicU64::new(0)]));
    let token_slot_ptr = token_slot.as_ptr() as usize;

    // spawn consumer。closure 捕获 key + token 槽。
    let join: task::Join<()> = task::closure(move || {
        // 等 producer accord 完 + 存 token。
        let _ = room::wait(key_token, WAIT_MS).expect("wait token");
        let token = unsafe { (*(token_slot_ptr as *const AtomicU64)).load(Ordering::Relaxed) };
        let hole = HolePie::from_token(token);

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

    // accord：源 pie 创建时自带 VEST 权；subset = READ ⊆ {R, W, VEST, BACK}。
    let token = pie.accord(join.id(), Permission::READ).expect("accord");
    token_slot[0].store(token, Ordering::Relaxed);
    let _ = room::wake(key_token).expect("wake token");

    for i in 0..N {
        let mut msg = [0u8; HOLE_MSG_LEN];
        msg[0] = i;
        while pie.push(&msg).is_err() {
            // slot 满 = consumer 还没 pull；等它 wake(key_empty)。
            let _ = room::wait(key_empty, WAIT_MS).expect("wait empty");
        }
        let _ = room::wake(key_ready).expect("wake ready");
    }

    // 等 consumer 跑完 16 轮再 seal。
    let _ = join.join();
    let _ = pie.seal();
    let _ = put("pair: done\n");
}
