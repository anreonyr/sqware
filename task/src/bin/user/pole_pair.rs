#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::Permission;
use task::core::unit;
use task::env::{io::put, mail::PolePie, room};

// pole_pair: 跨 Task 真共享 Pole（页级安全内存 + accord 派门闩 + fault isolation）。
//
// 主任务 = producer：unseal Pole、spawn consumer、accord 给 consumer（subset=READ）、
//   map 自己 space（自有 pie 全权 R|W），按 offset 写 16 字节，wake(key_data)，
//   等 consumer 读完 wake(key_done)，seal，打 done。
// 子任务 = consumer：凭 accord 返回的 token 重建 Pole、map 自己 space（**R-only**，
//   cap ⊆ 页表）、读 16 字节校验、wake(key_done)、**再写 R-only** → StorePageFault
//   → kernel 杀 task（fault isolation）；producer 不 join，靠 wait(key_done)。
//
// 双 key 协议（防自产自销）：key_data = producer→consumer "数据可读"；
//   key_done = consumer→producer "读完通知"。
//
// 跨模块不变量：producer accord 返回新 pie 的 token，存进 boxed 槽；consumer
// 读槽拿 token 用 `PolePie::from_token(token)` 重建句柄。

const N: u8 = 16;
const POLE_BYTES: usize = 4096;
// 握手等待 watchdog：协议保证 wake 必到（wait/wake 已闭环防丢唤醒），取 5s 大余量
// 防并发下瞬时延迟误报，同时有限有界——若协议未来又出错，5s 内即可暴露。
const WAIT: usize = 5_000;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("pole_pair\n");

    let pole = PolePie::unseal(POLE_BYTES).expect("unseal");

    // 两把 key 钉在堆（地址稳定、跨 task 共享）。
    let key_data: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_done: usize = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let token_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let token_slot_ptr = token_slot.as_ptr() as usize;

    // spawn consumer。closure 捕获 key + token 槽。consumer 末尾写 R-only 触发
    // fault isolation，被内核杀掉，**不会**走完 closure——producer 不可 join。
    let _join: unit::Join<()> = unit::closure(move || {
        // 等 producer accord + 存 token（先于第一次 wake key_data）。
        let _ = room::wait(key_data, WAIT).expect("wait data");
        let token = unsafe { (*(token_slot_ptr as *const AtomicUsize)).load(Ordering::Relaxed) };
        let pole = PolePie::from_token(token);
        let va = pole.map().expect("map r-only");
        let ptr = va as *const u8;
        // 读 16 字节不同 offset，校验。
        for i in 0..N {
            let b = unsafe { *ptr.add(i as usize) };
            if b == i {
                let _ = put("K\n");
            }
        }
        // 通知 producer "读完"——producer 拿这个 wake 决定何时 seal。
        let _ = room::wake(key_done).expect("wake done");
        // 测 cap ⊆ 页表：写 R-only 必 StorePageFault；内核走 fault isolation
        // 杀本 task（user 异常隔离，不 panic kernel）。ptr 是 const，强制 mut cast。
        unsafe { *(ptr as *mut u8).add(0) = 0xDE };
    });

    // accord：subset = READ（cap ⊆ 页表 → consumer 拿到的页面 R-only）。
    let token = pole.accord(_join.id(), Permission::READ).expect("accord");
    token_slot[0].store(token, Ordering::Relaxed);

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
    let _ = room::wait(key_done, WAIT).expect("wait done");

    // 收尾：不 join（consumer 已死）；直接 seal + done。
    let _ = pole.seal();
    let _ = put("pole_pair: done\n");
}
