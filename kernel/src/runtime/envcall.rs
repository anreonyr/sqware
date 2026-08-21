// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 调用号契约（a7 枚举）单一事实源在 `ubi::Ucall`，kernel/user 共用。
// 约定：a7 = 调用号，a0..a5 = 参数，返回值写回 a0/a1（Gprs::A0/A1）；
// 每个调用后 sepc += 4（Exit 除外——不返回）。时间语义统一以毫秒（Duration 边界）
// 表达（Sleep=4）；tick 计数（GetTicks=3）仅作兼容诊断，非时间单位。
// 分发只处理用户态陷阱：内核态 S-mode envcall 属内核 bug，走 trap 的
// "unhandled kernel exception" 分支。

use core::str;
use core::time::Duration;

use ubi::Ucall;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::put;
use crate::runtime::context::{Gprs, TrapContext};
use crate::runtime::trace::{self, EnvEvent, EventKind};
use crate::runtime::{clock, timer};
use crate::work::room::scheduler::{park, reap, running_team, starve, with_running_space};

/// envcall 分发（trap_handler 的 UserEnvCall 分支调用）。
///
/// 入参 frame = 当前任务用户帧；返回待恢复帧：Yield 返回调度器选出的下一任务帧，
/// Exit 返回后调用方不得再触碰 frame（其空间已在 reap 中回收）。
pub fn dispatch(frame: &mut TrapContext) -> *mut TrapContext {
    let number = frame.gpr.x(Gprs::A7); // a7 = 调用号
    let call =
        Ucall::try_from(number).unwrap_or_else(|_| panic!("invalid envcall number: {number}"));
    trace::note(EventKind::Env(EnvEvent::Call {
        call: number,
        arg: frame.gpr.x(Gprs::A0),
    }));
    frame.sepc += 4;
    match call {
        Ucall::Yield => return starve() as *mut TrapContext,
        Ucall::Write => {
            let len = frame.gpr.x(Gprs::A0);
            let ptr = frame.gpr.x(Gprs::A1);
            with_running_space(|space| {
                let va = VirtAddr::from_raw(ptr);
                if let Some((pa, flag)) = space.translate(va) {
                    if flag.intersects(PteFlags::R) {
                        unsafe {
                            put!("{}", str::from_raw_parts_mut(pa.as_usize() as *mut u8, len))
                        }
                    }
                } else {
                    frame.gpr.set_x(Gprs::A0, usize::MAX);
                }
            });
        }
        Ucall::Exit => {
            return reap() as *mut TrapContext;
        }
        Ucall::GetTicks => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        Ucall::Sleep => {
            // sepc 前进（唤醒恢复时从 ecall 之后继续）；任务被 park，返回下一帧。
            // a0 = 毫秒（Duration 边界；clock 按 timebase 换算成 deadline）
            return park(Duration::from_millis(frame.gpr.x(Gprs::A0) as u64)) as *mut TrapContext;
        }
        Ucall::ClockGetTime => {
            let up = clock::uptime();
            frame.gpr.set_x(Gprs::A0, up.as_secs() as usize);
            frame.gpr.set_x(Gprs::A1, up.subsec_nanos() as usize);
        }
        Ucall::HeapAllocate => {
            // a0 = 字节数（页对齐向上取整；内核 Window::allocate 要求页对齐）。
            // 当前运行任务空间经 with_running_space 借出（锁序 1→2→5 合法）。
            let size = frame.gpr.x(Gprs::A0).max(1).next_multiple_of(PAGE_SIZE);
            let addr = with_running_space(|s| s.heap_allocate(size));
            frame.gpr.set_x(
                Gprs::A0,
                match addr {
                    Ok(va) => va.as_usize(),
                    Err(_) => usize::MAX, // 负错误码（D1）：用户侧 (a0 as isize) < 0 判为 Err
                },
            );
        }
        Ucall::HeapDeallocate => {
            // a0 = 分配所得 VA，a1 = 字节数（与分配时同源页对齐；位图精确匹配）。
            let addr = frame.gpr.x(Gprs::A0);
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let ok = with_running_space(|s| s.heap_deallocate(VirtAddr::from_raw(addr), size));
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Spawn => {
            // a0 = 入口 VA（用户 trampoline），a1 = arg（闭包指针）。当前 team 建 U 任务。
            // running_team 放锁后由 TaskBuilder::spawn 逐段取锁建任务（不跨锁持有）。
            let entry = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let arg = frame.gpr.x(Gprs::A1);
            let team = running_team();
            let r = team.task().name("u-thread").entry(entry).arg(arg).spawn();
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(pa) => pa.as_usize(), // 任务句柄（trap 帧 PA，唯一；供用户记认）
                    Err(_) => usize::MAX,
                },
            );
        }
    };
    frame as *mut TrapContext
}
