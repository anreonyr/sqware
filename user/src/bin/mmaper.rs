#![no_std]
#![no_main]

use user::env::{exit, mmap, mmap_at, mprotect, munmap, put};

/// PTE 标志位（与内核 PteFlags 同构；mprotect 用）。
const PTE_V: usize = 1;
const PTE_R: usize = 2;
const PTE_U: usize = 16;

/// 打印 64 位十六进制（演示用；env 无数字打印机）。
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
    // SAFETY: buf 全为 ASCII。
    let _ = put(core::str::from_utf8(&buf).expect("ascii"));
}

// mmaper：mmap 三幕演示——高位大段懒匿名映射 / 触碰补帧 / 精确释放。
#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 幕 1：mmap 1 TiB。返回 VA 应落在高位（Sv39 ≈254 GiB / Sv48 ≈128 TiB /
    // Sv57 ≈64 PiB——>4 GiB 一出手可见）；未映射物理帧，零 RAM 成本。
    let region = match mmap(1usize << 40) {
        Ok(va) => va,
        Err(_) => {
            let _ = put("mmaper: mmap FAILED\n");
            exit()
        }
    };
    let _ = put("mmaper: mmap(1 TiB) @ ");
    put_hex(region);
    let _ = put("\n");

    // 幕 2：触碰 4 页——缺页懒分配零页帧，只有这 4 页被物化（稀疏帧经济）。
    for i in 0..4 {
        let p = (region + i * 4096) as *mut u8;
        // SAFETY: 页面映射由缺页在触碰时建立；写单字节无别名。
        unsafe { p.write_volatile(0x5A) };
    }
    for i in 0..4 {
        let p = (region + i * 4096) as *const u8;
        // SAFETY: 同上；读回验证。
        let v = unsafe { p.read_volatile() };
        debug_assert_eq!(v, 0x5A);
    }
    let _ = put("mmaper: touched 4 pages ok\n");

    // 幕 3：munmap 精确释放（含已触页帧归还 + PTE 清理）。
    match munmap(region, 1usize << 40) {
        Ok(()) => {
            let _ = put("mmaper: munmap ok\n");
        }
        Err(_) => {
            let _ = put("mmaper: munmap FAILED\n");
        }
    }

    // 幕 4：固定地址声明式懒映射（mmap_at = declare 常数侧：不占窗口、触碰补帧）
    // + mprotect 只读（未触页仅簿记同步）+ unmap 回收。
    const FIXED: usize = 0x8000; // 镜像基址 0x10000 之下，无窗口覆盖
    match mmap_at(FIXED, 4 * 4096) {
        Ok(va) if va == FIXED => {
            let _ = put("mmaper: mmap_at(0x8000, 16K) ok\n");
        }
        _ => {
            let _ = put("mmaper: mmap_at FAILED\n");
            exit()
        }
    }
    for i in 0..4 {
        let p = (FIXED + i * 4096) as *mut u8;
        // SAFETY: 声明区触碰经缺页补零页帧；写单字节无别名。
        unsafe { p.write_volatile(0x5A) };
    }
    let _ = put("mmaper: fixed touched 4 pages ok\n");

    // mprotect 只读（V|R|U，清 W）：已触页叶子 PTE 翻位、未触页仅簿记同步。
    if mprotect(FIXED, 4 * 4096, (PTE_V | PTE_R | PTE_U) as u64).is_ok() {
        let _ = put("mmaper: mprotect read-only ok\n");
    } else {
        let _ = put("mmaper: mprotect FAILED\n");
    }
    // 只读语义回归：已触页读回仍合法（写会缺页，此处不写）。
    let v = unsafe { (FIXED as *const u8).read_volatile() };
    debug_assert_eq!(v, 0x5A);

    match munmap(FIXED, 4 * 4096) {
        Ok(()) => {
            let _ = put("mmaper: munmap fixed ok\n");
        }
        Err(_) => {
            let _ = put("mmaper: munmap fixed FAILED\n");
        }
    }
    exit()
}
