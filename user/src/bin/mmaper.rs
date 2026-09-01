#![no_std]
#![no_main]

use user::env::{io::put, memory};

const PTE_V: usize = 1;
const PTE_R: usize = 2;
const PTE_U: usize = 16;

fn put_hex(v: usize) {
    let mut buf = [b'0'; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    let mut i = 17;
    let mut v = v;
    while i > 1 {
        buf[i] = b"0123456789abcdef"[v & 0xF];
        v >>= 4;
        i -= 1;
    }
    let _ = put(core::str::from_utf8(&buf).expect("ascii"));
}

// mmaper：mmap 三幕演示——高位懒映射 / 触碰补帧 / 精确释放。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let region = match memory::mmap(1usize << 40, None) {
        Ok(va) => va,
        Err(_) => {
            let _ = put("mmaper: mmap FAILED\n");
            return;
        }
    };
    let _ = put("mmaper: mmap(1 TiB) @ ");
    put_hex(region);
    let _ = put("\n");

    for i in 0..4 {
        let p = (region + i * 4096) as *mut u8;
        unsafe { p.write_volatile(0x5A) };
    }
    for i in 0..4 {
        let p = (region + i * 4096) as *const u8;
        let v = unsafe { p.read_volatile() };
        debug_assert_eq!(v, 0x5A);
    }
    let _ = put("mmaper: touched 4 pages ok\n");

    match memory::munmap(region, 1usize << 40) {
        Ok(()) => { let _ = put("mmaper: munmap ok\n"); }
        Err(_) => { let _ = put("mmaper: munmap FAILED\n"); }
    }

    const FIXED: usize = 0x8000;
    match memory::mmap(4 * 4096, Some(FIXED)) {
        Ok(va) if va == FIXED => { let _ = put("mmaper: mmap_at(0x8000, 16K) ok\n"); }
        _ => {
            let _ = put("mmaper: mmap_at FAILED\n");
            return;
        }
    }
    for i in 0..4 {
        let p = (FIXED + i * 4096) as *mut u8;
        unsafe { p.write_volatile(0x5A) };
    }
    let _ = put("mmaper: fixed touched 4 pages ok\n");

    if memory::mprotect(FIXED, 4 * 4096, (PTE_V | PTE_R | PTE_U) as u64).is_ok() {
        let _ = put("mmaper: mprotect read-only ok\n");
    } else {
        let _ = put("mmaper: mprotect FAILED\n");
    }
    let v = unsafe { (FIXED as *const u8).read_volatile() };
    debug_assert_eq!(v, 0x5A);

    match memory::munmap(FIXED, 4 * 4096) {
        Ok(()) => { let _ = put("mmaper: munmap fixed ok\n"); }
        Err(_) => { let _ = put("mmaper: munmap fixed FAILED\n"); }
    }
}