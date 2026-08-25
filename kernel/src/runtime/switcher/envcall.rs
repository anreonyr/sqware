// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 调用号契约（a7 枚举）单一事实源在 `ubi::Ucall`。
// 约定：a7 = 调用号，a0..a5 = 参数，返回值写回 a0/a1（Gprs::A0/A1）；
// 每个调用后 sepc += 4（Exit 除外——不返回）。时间语义统一以毫秒（Duration 边界）
// 表达（Sleep=4）；tick 计数（GetTicks=3）仅作兼容诊断，非时间单位。

use core::time::Duration;

use ubi::Ucall;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::scheduler::{park, reap, running_team, starve, with_running_space};
use crate::work::unit::space::MapKind;

/// envcall 分发。
///
/// 入参 frame = 当前任务用户帧；返回待恢复帧：Yield 返回下一任务帧，
/// Exit 返回后调用方不得再触碰 frame。
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
            // 以后接入 operator: file system adaptor
            let len = frame.gpr.x(Gprs::A0);
            let ptr = frame.gpr.x(Gprs::A1);
            let ok = with_running_space(|space| crate::console::write_in(space, ptr, len));
            if !ok {
                frame.gpr.set_x(Gprs::A0, usize::MAX);
            }
        }
        Ucall::Exit => {
            return reap() as *mut TrapContext;
        }
        Ucall::GetTicks => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        Ucall::Sleep => {
            // sepc 前进（唤醒恢复时从 ecall 之后继续）。
            // a0 = 毫秒（Duration 边界；换算按 timebase 进行）
            return park(Duration::from_millis(frame.gpr.x(Gprs::A0) as u64)) as *mut TrapContext;
        }
        Ucall::ClockGetTime => {
            let up = clock::uptime();
            frame.gpr.set_x(Gprs::A0, up.as_secs() as usize);
            frame.gpr.set_x(Gprs::A1, up.subsec_nanos() as usize);
        }
        Ucall::HeapAllocate => {
            // a0 = 字节数（页对齐向上取整——分配需页对齐）。
            // 经 with_running_space 借出当前任务空间（锁序 1→2→5 合法）。
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
            // a0 = 入口 VA（用户 trampoline），a1 = arg（闭包指针），a2 = 栈大小
            // （0 = 缺省 TASK_STACK_SIZE）。当前 team 内建 U 任务。
            let entry = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let arg = frame.gpr.x(Gprs::A1);
            let stack = frame.gpr.x(Gprs::A2);
            let team = running_team();
            let mut builder = team.task().name("u-thread").entry(entry).arg(arg);
            if stack > 0 {
                builder = builder.stack(stack);
            }
            let r = builder.spawn();
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(pa) => pa.as_usize(), // 任务句柄（trap 帧 PA，唯一；供用户记认）
                    Err(_) => usize::MAX,
                },
            );
        }
        Ucall::Panic => {
            // 用户主动 panic：a0 = 呼叫人指定的关联码。
            panic!("user-initiated panic (code {:#x})", frame.gpr.x(Gprs::A0));
        }
        Ucall::Mmap => {
            // a0 = 字节数（页对齐）；a2 = 期望 VA，0 = 窗口自选高位。a2 ≠ 0 走
            // 声明式固定地址懒映射（declare 登记常数侧：触碰经既有缺页补零页帧，
            // 删除经 Munmap 回退 [`Space::unmap`] 摘整段）。
            let size = frame.gpr.x(Gprs::A0).max(1).next_multiple_of(PAGE_SIZE);
            let fixed = frame.gpr.x(Gprs::A2);
            let va = with_running_space(|s| {
                if fixed == 0 {
                    s.mmap(size)
                } else {
                    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
                    s.declare(VirtAddr::from_raw(fixed), size, flags, MapKind::Anonymous)
                        .map(|()| VirtAddr::from_raw(fixed))
                }
            });
            frame.gpr.set_x(
                Gprs::A0,
                match va {
                    Ok(va) => va.as_usize(),
                    Err(_) => usize::MAX, // D1：负值错误码
                },
            );
        }
        Ucall::Munmap => {
            // a0 = 映射 VA，a1 = 字节数（与 mmap 同源页对齐）。窗口区域经 munmap
            // 精确匹配；固定地址声明区（常数侧登记）回退 Space::unmap 摘整段。
            // 未命中任何映射仍返回错误。
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let ok = with_running_space(|s| {
                if s.munmap(addr, size) {
                    true
                } else if s.resolve_kind(addr).is_some() {
                    // 声明区（或窗口子区间的近似覆盖）：PTE 清 + 整段摘除；窗口
                    // 位图槽不归还——边界拆分留待 mprotect 后端细化。
                    s.unmap(addr, size);
                    true
                } else {
                    false
                }
            });
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Mprotect => {
            // a0 = 映射 VA，a1 = 字节数（页对齐），a2 = 新权限（PteFlags 位）。
            // 懒区感知：已触页当场翻叶子 PTE，未触页仅同步簿记（Map.flags）。
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let flags = PteFlags::from_bits_truncate(frame.gpr.x(Gprs::A2) as u64);
            let ok = with_running_space(|s| s.mprotect(addr, size, flags).is_ok());
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
    };
    frame as *mut TrapContext
}
