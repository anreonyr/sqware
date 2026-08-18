#![no_std]
//! 用户态程序共享的 envcall（环境调用）辅助。
//!
//! 调用号须与内核 `work::envcall` 对齐：YIELD/WRITE/EXIT/GET_TICKS/SLEEP。
//! 经 U 态 ecall 进内核，由 trap_handler 的 UserEnvCall 分支分发。

/// 环境调用号。
pub mod sys {
    pub const YIELD: usize = 0;
    pub const WRITE: usize = 1;
    pub const EXIT: usize = 2;
    pub const GET_TICKS: usize = 3;
    pub const SLEEP: usize = 4;
}

/// 单字符输出（SYS_WRITE；a0 = 字符，结果回写 a0）。
pub fn put(ch: u8) {
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "mv a0, {ch}",
            "ecall",
            c = const sys::WRITE,
            ch = in(reg) ch as usize,
            options(nostack),
        );
    }
}

/// 退出当前任务（SYS_EXIT；不返回）。
pub fn exit() -> ! {
    unsafe {
        core::arch::asm!(
            "li a7, {c}",
            "ecall",
            c = const sys::EXIT,
            options(noreturn, nostack),
        );
    }
}
