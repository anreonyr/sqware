#![no_std]
#![no_main]

extern crate alloc;

use user::core::task;
use user::core::tls;
use user::env::{io::put, room::exit};

// tlser：TLS 地基验收——主线程 + 子线程各自独立 TLS 块。

const SLOT: usize = 0;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let _ = put("tlser\n");
    let mine = tls::base();

    let a = task::closure(|| {
        let base = tls::base();
        unsafe { (base as *mut usize).add(SLOT).write(0xA5A5_A5A5) };
        (base, 0xA5A5_A5A5usize)
    });
    let (ba, va) = a.join();
    let _ = put("P\n");

    let b = task::closure(|| {
        let base = tls::base();
        unsafe { (base as *mut usize).add(SLOT).write(0x5A5A_5A5A) };
        (base, 0x5A5A_5A5Ausize)
    });
    let (bb, vb) = b.join();
    let _ = put("P\n");

    let ok = ba != mine && bb != mine && ba != bb && va == 0xA5A5_A5A5 && vb == 0x5A5A_5A5A;
    if ok {
        let _ = put("tlser: ok\n");
    } else {
        let _ = put("tlser: FAILED\n");
    }
    exit()
}