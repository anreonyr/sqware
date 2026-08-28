#![no_std]
#![no_main]

extern crate alloc;

use user::env::{self, put};
use user::task;
use user::tls;

// tlser：TLS 地基验收——主线程 + 子线程各自独立 TLS 块。
//
// 验证点：
//   1. `tls::base()`（= tp）在装配后指向本线程块，各线程互异；
//   2. 各线程经自己块内偏移写/读，互不串写。

/// TLS 块内自订偏移：本线程"私号"槽（块首 usize）。
const SLOT: usize = 0;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("tlser\n").ok();
    let mine = tls::base();

    // 子线程 A：写私号 → 回传 (base, 私号)
    let a = task::closure(|| {
        let base = tls::base();
        unsafe { (base as *mut usize).add(SLOT).write(0xA5A5_A5A5) };
        (base, 0xA5A5_A5A5usize)
    });
    let (ba, va) = a.join();
    put("P\n").ok();

    // 子线程 B：写另一私号 → 回传 (base, 私号)
    let b = task::closure(|| {
        let base = tls::base();
        unsafe { (base as *mut usize).add(SLOT).write(0x5A5A_5A5A) };
        (base, 0x5A5A_5A5Ausize)
    });
    let (bb, vb) = b.join();
    put("P\n").ok();

    // 验收：三块互异、私号互不串写
    let ok = ba != mine && bb != mine && ba != bb && va == 0xA5A5_A5A5 && vb == 0x5A5A_5A5A;
    if ok {
        put("tlser: ok\n").ok();
    } else {
        put("tlser: FAILED\n").ok();
    }
    env::exit()
}
