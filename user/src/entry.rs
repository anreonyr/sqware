//! 共享入口（user-bin 引导 + panic 处理）。

use core::arch::global_asm;

use crate::env::{control::backtrace, io::put, room::exit};

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call tls_bootstrap",
    "    call main",
    "    call exit_trampoline", // main 返回（理论上 !，兜底）→ room::exit
    "1: j 1b",                  // ec 返回则兜底循环
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { crate::core::tls::bootstrap() }
}

#[unsafe(no_mangle)]
extern "C" fn exit_trampoline() -> ! {
    exit()
}

// panic_handler 路径禁忌：不能走 writeln!/format!（潜在分配 → 双重 panic）。
// 走直接 put 字符串 + 整数的十进制逐位写。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = put("user paniced\n");
    if let Some(loc) = info.location() {
        let _ = put("  at ");
        let _ = put(loc.file());
        let _ = put(":");
        let mut n = loc.line();
        if n == 0 {
            let _ = put("0");
        } else {
            let mut buf = [0u8; 20];
            let mut i = 20;
            while n > 0 && i > 0 {
                i -= 1;
                buf[i] = b'0' + (n % 10) as u8;
                n /= 10;
            }
            let _ = put(core::str::from_utf8(&buf[i..]).unwrap_or("?"));
        }
        let _ = put("\n");
    }
    // 崩溃时采样调用链（pc 地址数组）并逐帧打印裸 hex——panic 现场零分配。
    // 注意：回溯采样的是当前调用点（panic handler → backtrace）的现场，业务帧
    // 在链更深处；仍具诊断价值（能看到触发 panic 前后的调用链轮廓）。
    let _ = put("backtrace:\n");
    let mut fr = [0usize; 32];
    let n = backtrace(&mut fr);
    for i in 0..n {
        put_hex_usize(i, fr[i]);
    }
    exit()
}

/// 无分配打印 `#[i] pc=0x…`（panic 现场禁忌 format!/writeln!——潜在分配双重 panic）。
fn put_hex_usize(idx: usize, v: usize) {
    let _ = put("#");
    let mut d = [0u8; 4];
    let mut di = 4;
    let mut n = idx;
    while n > 0 && di > 0 {
        di -= 1;
        d[di] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let _ = put(core::str::from_utf8(&d[di..]).unwrap_or("0"));
    let _ = put(" pc=0x");
    // 逐十六进制位（16 个 4 位半字节），高位 0 跳过。
    let mut started = false;
    for shift in (0..64).rev().step_by(4) {
        let nib = (v >> shift) & 0xF;
        if nib != 0 || started || shift == 0 {
            started = true;
            let c = match nib {
                0..=9 => b'0' + nib as u8,
                10..=15 => b'a' + (nib as u8 - 10),
                _ => b'?',
            };
            let _ = put(core::str::from_utf8(&[c]).unwrap_or("?"));
        }
    }
    let _ = put("\n");
}
