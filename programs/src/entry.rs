//! 共享入口（镜像程序引导 + panic 处理）：四个 `bin/` 共用，故住在 lib 面。

use core::arch::global_asm;

use runtime::env::{io::put, room::exit};

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call save_args", // a0/a1 = 启动参数区（Spawn 写入）——必须在任何调用前保存
    "    call tls_bootstrap",
    "    call main",
    "    call exit_trampoline", // main 返回（理论上 !，兜底）→ room::exit
    "1: j 1b",                  // ec 返回则兜底循环
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { runtime::core::tls::bootstrap() }
}

#[unsafe(no_mangle)]
extern "C" fn exit_trampoline() -> ! {
    exit()
}

/// 域内 panic：留**只有本侧知道**的那一半，然后把处置交给内核。
///
/// 分工的判据是"谁知道这件事"：
///
/// | 事实 | 谁知道 | 谁打 |
/// |---|---|---|
/// | 源码位置（`file:line`） | 只有用户侧 | 本函数（`put`，一行） |
/// | 是哪个域、tid、原因码 | 只有内核 | 内核（`RoomEvent::Exit{reason}`） |
/// | 该域是否已收尾 | 只有内核 | 内核（`RoomEvent::PanicDeclared` → `reap` → `wipe`） |
///
/// 收尾走 [`exit`]（= `RoomCall::Reap { reason: 0 }`）：**内核不需要知道"这是一次
/// panic"**——那是域的判断，内核只需要"这个任务不再续跑"。本仓一度另立了
/// `ControlCall::Panic` 走这条路，已收回：它把域的策略写进了 ABI，并让
/// "任务终止"这条不变量在 ABI 里有两个出口。
///
/// panic 现场禁忌（照旧）：**不能**走 `format!` / `writeln!`（潜在分配 → 双重 panic）。
/// 故本函数只有字面量 `put` 与整数逐位写。
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
    // 收尾走**通用终止原语**（`Reap`，原因码 0 = 自愿/正常）：内核不需要知道"这是
    // 一次 panic"——那是域的判断。位置那条事实已经在上面写出去了，域能说的就这些；
    // 剩下的（是哪个 tid、域是否已收尾）归内核，`RoomEvent::Exit` 会带上原因码。
    // 各 bin 的启动握手失败分支给的是各自的编号（`1`、`2`…），两者在 trace 里分得开。
    exit()
}
