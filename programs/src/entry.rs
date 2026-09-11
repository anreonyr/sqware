//! 共享入口（镜像程序引导 + panic 处理）：四个 `bin/` 共用，故住在 lib 面。

use core::arch::global_asm;

use runtime::env::room::exit_with;

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
    exit_with(EXIT_OK)
}

/// 自愿结束的原因码（域自己的编号空间的 0 号）。
pub const EXIT_OK: usize = 0;

/// 域 panic 的原因码——**域自己的诊断编号**，取高位段、与各 bin 的启动握手编号
/// （小整数）不重叠：读 trace 的人一眼能分出"启动没走通"与"域自己炸了"。
pub const EXIT_PANIC: usize = 0xFFFF_FF01;

/// 域内 panic：**不打印**，把处置交给内核（`docs/driver.md` §3.3.5）。
///
/// 一个死掉的域还能打印，前提是服务、门闩、槽全都活着——**那是错的依赖方向**：
/// 域正在告诉你它算不下去了，却要它先去求一条活路。故这里一行字都不写。
///
/// 那"哪里崩的"怎么办——**靠内核那头**：
/// - `Reap { reason: EXIT_PANIC }` 进 trace 的 `RoomEvent::Exit`（谁、何时、为何），
/// - panic 现场（sepc/sp/寄存器）由内核在自己的故障路径上留痕，
/// - `sepc` 事后经 initrd 符号表解析成 `file:line`。
///
/// 这条路的前提（**要修**）是 `symbol()` 还只是十六进制桩：符号化不落地，"不打印"
/// 就等于"看不见"。它记在 `docs/driver.md` §12 的"要修"一栏。
///
/// panic 现场禁忌（照旧）：**不能**走 `format!` / `writeln!`（潜在分配 → 双重 panic）。
/// 现在连字面量也不写——本函数只剩一条 `Reap`。
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit_with(EXIT_PANIC)
}
