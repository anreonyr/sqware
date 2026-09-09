#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};

use env::Permission;
use task::core::unit;
use task::env::{
    io::put,
    mail::{HolePie, HOLE_MTU_MAX},
    room,
};

// hole_pair: 跨 Task 真共享 Hole（accord 派门闩 + 多 key 同步）。
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
// 握手等待 watchdog：协议保证 wake 必到（wait/wake 已闭环防丢唤醒），取 5s 大余量
// 防并发下瞬时延迟误报，同时有限有界——若协议未来又出错，5s 内即可暴露。
const WAIT: usize = 5_000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("hole_pair\n");

    let pie = HolePie::unseal(HOLE_MTU_MAX).expect("unseal");

    // 三把 key 钉在堆（地址稳定、跨 task 共享）。
    let key_token: usize = Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;
    let key_ready: usize = Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;
    let key_empty: usize = Box::leak(Box::new([0u8; HOLE_MSG_LEN])).as_ptr() as usize;

    let token_slot: &'static [AtomicU64; 1] = Box::leak(Box::new([AtomicU64::new(0)]));
    let token_slot_ptr = token_slot.as_ptr() as usize;

    // spawn consumer。closure 捕获 key + token 槽。
    let join: unit::Join<()> = unit::closure(move || {
        // 等 producer accord 完 + 存 token。
        let _ = room::wait(key_token, WAIT).expect("wait token");
        let token = unsafe { (*(token_slot_ptr as *const AtomicU64)).load(Ordering::Relaxed) };
        let hole = HolePie::from_token(token);

        for i in 0..N {
            let _ = room::wait(key_ready, WAIT).expect("wait ready");
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
            let _ = room::wait(key_empty, WAIT).expect("wait empty");
        }
        let _ = room::wake(key_ready).expect("wake ready");
    }

    // 等 consumer 跑完 16 轮再 seal。
    let _ = join.join();
    let _ = pie.seal();
    let _ = put("hole_pair: done\n");
}
