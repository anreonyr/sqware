#![no_std]
//! 用户态程序共享的 envcall（环境调用）辅助。
//!
//! 调用号须与内核 `work::envcall` 对齐：YIELD/WRITE/EXIT/GET_TICKS/SLEEP。
//! 经 U 态 ecall 进内核，由 trap_handler 的 UserEnvCall 分支分发。
//! 每个辅助都显式列出被 ecall 践踏的 a0/a7，避免编译器把本地量放进这些寄存器。

/// 环境调用号。
pub mod sys {
    pub const YIELD: usize = 0;
    pub const WRITE: usize = 1;
    pub const EXIT: usize = 2;
    pub const GET_TICKS: usize = 3;
    pub const SLEEP: usize = 4;
}

/// 主动让出处理器（SYS_YIELD）。
pub fn yield_() {
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "ecall",
            c = const sys::YIELD,
            out("a0") _,
            out("a7") _,
            options(nostack),
        );
    }
}

/// 单字符输出（SYS_WRITE；a0 = 字符）。
pub fn put(ch: u8) {
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "mv a0, {ch}",
            "ecall",
            c = const sys::WRITE,
            ch = in(reg) ch as usize,
            out("a0") _,
            out("a7") _,
            options(nostack),
        );
    }
}

/// 退出当前任务（SYS_EXIT；不返回）。
pub fn exit() -> ! {
    // ENV_EXIT 不返回（内核随后调度别的任务），asm 不带 noreturn、事后显式 diverge。
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "ecall",
            c = const sys::EXIT,
            out("a0") _,
            out("a7") _,
            options(nostack),
        );
    }
    unsafe { core::hint::unreachable_unchecked() }
}

/// 读定时器 tick 计数（SYS_GET_TICKS；结果回写 a0）。
pub fn get_ticks() -> usize {
    let t: usize;
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "ecall",
            c = const sys::GET_TICKS,
            out("a0") t,
            out("a7") _,
            options(nostack),
        );
    }
    t
}

/// 睡眠 `ticks` 个量子（SYS_SLEEP）。
pub fn sleep(ticks: usize) {
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "mv a0, {ticks}",
            "ecall",
            c = const sys::SLEEP,
            ticks = in(reg) ticks,
            out("a0") _,
            out("a7") _,
            options(nostack),
        );
    }
}
