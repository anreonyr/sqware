// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 约定：a7 = 调用号（枚举，禁止裸数字），a0..a5 = 参数，返回值写回 a0
// （frame.gpr[10]）；每个调用后 sepc += 4（Exit 除外——不返回）。分发只处理
// 用户态陷阱：内核态 S-mode envcall 属内核 bug，走 trap 的 "unhandled kernel
// exception" 分支（Exception::SupervisorEnvCall）。

use crate::put;
use crate::runtime::context::TrapContext;
use crate::runtime::trap::ticks;
use crate::task::{exit_current, tick};

/// envcall 调用号。
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Envcall {
    /// 主动让出处理器（round-robin 轮转，定时器抢占的主动版）。
    Yield = 0,
    /// 输出单字符（a0 = 字符码；最小可用，缓冲区写留待延伸）。
    Write = 1,
    /// 退出当前任务（不返回）。
    Exit = 2,
    /// 读取定时器 tick 计数（返回值写 a0）。
    GetTicks = 3,
}

impl TryFrom<usize> for Envcall {
    type Error = ();

    fn try_from(number: usize) -> Result<Self, ()> {
        match number {
            0 => Ok(Self::Yield),
            1 => Ok(Self::Write),
            2 => Ok(Self::Exit),
            3 => Ok(Self::GetTicks),
            _ => Err(()),
        }
    }
}

/// envcall 分发（trap_handler 的 UserEnvCall 分支调用）。
///
/// 入参 frame = 当前任务用户帧；返回待恢复帧：Yield/Exit 返回调度器选出的
/// 下一任务帧，其余返回入参帧。Exit 返回后调用方不得再触碰 frame（其空间
/// 已在 exit_current 中回收）。
pub fn dispatch(frame: &mut TrapContext) -> *mut TrapContext {
    let number = frame.gpr[17]; // a7
    let call =
        Envcall::try_from(number).unwrap_or_else(|_| panic!("invalid envcall number: {number}"));
    match call {
        Envcall::Yield => {
            frame.sepc += 4;
            tick() as *mut TrapContext
        }
        Envcall::Write => {
            frame.sepc += 4;
            let ch = frame.gpr[10] as u8 as char; // a0
            put!("{ch}");
            frame as *mut TrapContext
        }
        Envcall::Exit => {
            // 不做 sepc += 4：任务不再恢复；frame 指向的空间随即在 exit_current 中回收
            exit_current() as *mut TrapContext
        }
        Envcall::GetTicks => {
            frame.sepc += 4;
            frame.gpr[10] = ticks();
            frame as *mut TrapContext
        }
    }
}
