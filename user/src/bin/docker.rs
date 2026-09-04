#![no_std]
#![no_main]

extern crate alloc;

use user::env::io::put;
use user::env::mail::PolePie;

// docker: Pole 共享内存邮路单端压力测试——主任务开 Pole、map 自己 space，
// 写 64 字节、读回校验。
// （跨 Task 共享场景：N pier 各自凭 vest 来的 Pole<R/W> pie 写，quay 自留 pie
// 读——权限子集即隔离。本 demo 跑单端验证 Pole 端到端通路。）

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("docker\n");

    let pole = PolePie::open(4096).expect("pole open");
    let va = pole.map().expect("map");
    let ptr = va as *mut u8;

    // 写 64 字节
    for i in 0..64u8 {
        unsafe { *ptr.add(i as usize) = i; }
    }
    // 校验
    let mut ok = true;
    for i in 0..64u8 {
        let b = unsafe { *ptr.add(i as usize) };
        if b != i {
            ok = false;
        }
    }
    if ok {
        for _ in 0..64 {
            let _ = put("D\n");
        }
    } else {
        let _ = put("docker: mismatch!\n");
    }
    let _ = put("docker: done\n");
}