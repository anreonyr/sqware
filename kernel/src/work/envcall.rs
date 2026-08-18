// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 调用号契约（a7 枚举）单一事实源在 `ubi::Ucall`，kernel/user 共用（见 ubi）。
// 约定：a7 = 调用号，a0..a5 = 参数，返回值写回 a0/a1（frame.gpr[10]/gpr[11]）；
// 每个调用后 sepc += 4（Exit 除外——不返回）。时间语义统一以毫秒（Duration 边界）
// 表达（Sleep=4）；tick 计数（GetTicks=3）仅作兼容诊断，非时间单位。
// 分发只处理用户态陷阱：内核态 S-mode envcall 属内核 bug，走 trap 的
// "unhandled kernel exception" 分支。

use core::time::Duration;

use ubi::Ucall;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::put;
use crate::runtime::{clock, timer};
use crate::runtime::context::TrapContext;
use crate::work::scheduler::{park, reap, starve, with_running_space};

/// envcall 分发（trap_handler 的 UserEnvCall 分支调用）。
///
/// 入参 frame = 当前任务用户帧；返回待恢复帧：Yield 返回调度器选出的下一任务帧，
/// Exit 返回后调用方不得再触碰 frame（其空间已在 reap 中回收）。
pub fn dispatch(frame: &mut TrapContext) -> *mut TrapContext {
    let number = frame.gpr[17]; // a7
    let call =
        Ucall::try_from(number).unwrap_or_else(|_| panic!("invalid envcall number: {number}"));
    match call {
        Ucall::Yield => {
            frame.sepc += 4;
            starve() as *mut TrapContext
        }
        Ucall::Write => {
            frame.sepc += 4;
            let ch = frame.gpr[10] as u8 as char; // a0
            put!("{ch}");
            frame as *mut TrapContext
        }
        Ucall::Exit => {
            // 不做 sepc += 4：任务不再恢复；frame 指向的空间随即在 reap 中回收
            reap() as *mut TrapContext
        }
        Ucall::GetTicks => {
            frame.sepc += 4;
            frame.gpr[10] = timer::ticks() as usize;
            frame as *mut TrapContext
        }
        Ucall::Sleep => {
            // sepc 前进（唤醒恢复时从 ecall 之后继续）；任务被 park，返回下一帧。
            // a0 = 毫秒（Duration 边界；clock 按 timebase 换算成 deadline）
            frame.sepc += 4;
            park(Duration::from_millis(frame.gpr[10] as u64)) as *mut TrapContext
        }
        Ucall::ClockGetTime => {
            frame.sepc += 4;
            let up = clock::uptime();
            frame.gpr[10] = up.as_secs() as usize;
            frame.gpr[11] = up.subsec_nanos() as usize;
            frame as *mut TrapContext
        }
        Ucall::HeapAllocate => {
            // a0 = 字节数（页对齐向上取整；内核 Window::allocate 要求页对齐）。
            // 当前运行任务空间经 with_running_space 借出（锁序 1→2→5 合法）。
            frame.sepc += 4;
            let size = frame.gpr[10].max(1).next_multiple_of(PAGE_SIZE);
            let addr = with_running_space(|s| s.heap_allocate(size));
            frame.gpr[10] = match addr {
                Ok(va) => va.as_usize(),
                Err(_) => usize::MAX, // 负错误码（D1）：用户侧 (a0 as isize) < 0 判为 Err
            };
            frame as *mut TrapContext
        }
        Ucall::HeapDeallocate => {
            // a0 = 分配所得 VA，a1 = 字节数（与分配时同源页对齐；位图精确匹配）。
            frame.sepc += 4;
            let addr = frame.gpr[10];
            let size = frame.gpr[11].max(1).next_multiple_of(PAGE_SIZE);
            let ok = with_running_space(|s| {
                s.heap_deallocate(VirtAddr::from_raw(addr), size)
            });
            frame.gpr[10] = if ok { 0 } else { usize::MAX };
            frame as *mut TrapContext
        }
    }
}
