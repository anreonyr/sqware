// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 约定：a7 = 调用号，a0..a5 = 参数，返回值写回 a0/a1（Gprs::A0/A1）；
// 每个调用后 sepc += 4（Reap 除外——不返回）。时间语义统一以毫秒（Duration 边界）
// 表达（Park=4）；tick 计数（Ticks=3）仅作兼容诊断，非时间单位。
// 调用名与调度词族（conductor）同词：Starve/Park/Reap 即 utask 三面服务。

use core::time::Duration;

use ubi::Ucall;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::conductor::utask::{park, reap, starve};
use crate::work::unit::space::MapKind;
use crate::work::unit::task::TaskIdent;

/// envcall 分发。
///
/// 入参 frame = 当前任务用户帧；返回待恢复帧：Starve/Park 返回下一任务帧，
/// Reap 返回后调用方不得再触碰 frame。
pub fn dispatch(frame: &mut TrapContext, ident: &TaskIdent) -> *mut TrapContext {
    let number = frame.gpr.x(Gprs::A7); // a7 = 调用号
    let call =
        Ucall::try_from(number).unwrap_or_else(|_| panic!("invalid envcall number: {number}"));
    trace::note(EventKind::Env(EnvEvent::Call {
        call: number,
        arg: frame.gpr.x(Gprs::A0),
    }));
    frame.sepc += 4;
    match call {
        Ucall::Starve => return starve() as *mut TrapContext,
        Ucall::Put => {
            // 以后接入 operator: file system adaptor
            let len = frame.gpr.x(Gprs::A0);
            let ptr = frame.gpr.x(Gprs::A1);
            let ok = crate::console::write_in(&ident.team.space, ptr, len);
            if !ok {
                frame.gpr.set_x(Gprs::A0, usize::MAX);
            }
        }
        Ucall::Reap => {
            return reap() as *mut TrapContext;
        }
        Ucall::Ticks => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        Ucall::Park => {
            // sepc 前进（唤醒恢复时从 ecall 之后继续）。
            // a0 = 毫秒（Duration 边界；换算按 timebase 进行）
            return park(Duration::from_millis(frame.gpr.x(Gprs::A0) as u64)) as *mut TrapContext;
        }
        Ucall::Clock => {
            let up = clock::uptime();
            frame.gpr.set_x(Gprs::A0, up.as_secs() as usize);
            frame.gpr.set_x(Gprs::A1, up.subsec_nanos() as usize);
        }
        Ucall::Allocate => {
            // a0 = 字节数（页对齐向上取整——分配需页对齐）。
            // 直取当前任务空间（ident 无锁；锁序仅 L2→L5，不再经调度锁）；
            // 堆分配即时物化帧 → with_flush（PTE 变更后刷本空间 TLB）。
            let size = frame.gpr.x(Gprs::A0).max(1).next_multiple_of(PAGE_SIZE);
            let addr = {
                let s = &ident.team.space;
                let r = s.with_flush(|inner| {
                    inner
                        .heap
                        .as_mut()
                        .ok_or(MapError::NoRegion)?
                        .allocate(&mut inner.durable, size)
                });
                // 护栏事件：用户堆活块入账（alloc-site；用户侧清零语义，不 poison/canary）。
                // 键 = fence::key（页索引编码）：多个空间共享同 VA，须并入空间身份——
                // (asid<<44)|(va>>12) 单射（asid<2^16, va<2^56）。
                if let Ok(va) = r {
                    crate::memory::allocator::fence::on_alloc(
                        crate::memory::allocator::fence::key(s.asid(), va.as_usize()),
                        size,
                        crate::memory::allocator::fence::OwnerKind::UserHeap,
                    );
                }
                r
            };
            frame.gpr.set_x(
                Gprs::A0,
                match addr {
                    Ok(va) => va.as_usize(),
                    Err(_) => usize::MAX, // 负错误码（D1）：用户侧 (a0 as isize) < 0 判为 Err
                },
            );
        }
        Ucall::Deallocate => {
            // a0 = 分配所得 VA，a1 = 字节数（与分配时同源页对齐；区间精确匹配）。
            let addr = frame.gpr.x(Gprs::A0);
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                let freed = s.with_flush(|inner| match inner.heap.as_mut() {
                    Some(h) => h.deallocate(&mut inner.durable, VirtAddr::from_raw(addr), size),
                    None => false,
                });
                // 护栏事件：用户堆活块注销（无账 = 悬垂/双释放现行；键与分配相同）。
                if freed {
                    crate::memory::allocator::fence::on_free(
                        crate::memory::allocator::fence::key(s.asid(), addr),
                        size,
                        crate::memory::allocator::fence::OwnerKind::UserHeap,
                    );
                }
                freed
            };
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Spawn => {
            // a0 = 入口 VA（用户 trampoline），a1 = arg（闭包指针），a2 = 栈大小
            // （0 = 缺省 TASK_STACK_SIZE）。当前 team 内建 U 任务。
            let entry = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let arg = frame.gpr.x(Gprs::A1);
            let stack = frame.gpr.x(Gprs::A2);
            let team = ident.team.clone();
            let mut builder = team.task().name("u-thread").entry(entry).arg(arg);
            if stack > 0 {
                builder = builder.stack(stack);
            }
            let r = builder.spawn();
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(id) => id, // 任务号（全局唯一句柄；供用户记认）
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
            // 声明式固定地址懒映射。
            let size = frame.gpr.x(Gprs::A0).max(1).next_multiple_of(PAGE_SIZE);
            let fixed = frame.gpr.x(Gprs::A2);
            let va = {
                let s = &ident.team.space;
                if fixed == 0 {
                    // 窗口自选高位肯定有堆窗（若非 → NoRegion）；懒映射不改页表 → with
                    s.with(|inner| {
                        inner
                            .heap
                            .as_mut()
                            .ok_or(MapError::NoRegion)?
                            .mmap(size)
                    })
                } else {
                    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
                    s.declare(VirtAddr::from_raw(fixed), size, flags, MapKind::Anonymous)
                        .map(|()| VirtAddr::from_raw(fixed))
                }
            };
            frame.gpr.set_x(
                Gprs::A0,
                match va {
                    Ok(va) => va.as_usize(),
                    Err(_) => usize::MAX, // D1：负值错误码
                },
            );
        }
        Ucall::Munmap => {
            // a0 = 映射 VA，a1 = 字节数（与 mmap 同源页对齐）。窗口区域精确匹配，
            // 固定地址声明区回退整段摘除；未命中返回错误。
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                if s.with_flush(|inner| match inner.heap.as_mut() {
                    Some(h) => h.munmap(&mut inner.durable, addr, size),
                    None => false,
                }) {
                    true
                } else if s.resolve_kind(addr).is_some() {
                    // 声明区（或窗口子区间的近似覆盖）：PTE 清 + 整段摘除；窗口
                    // 槽不归还——边界拆分留待 mprotect 后端细化。
                    s.unmap(addr, size);
                    true
                } else {
                    false
                }
            };
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Mprotect => {
            // a0 = 映射 VA，a1 = 字节数（页对齐），a2 = 新权限（PteFlags 位）。
            // 懒区感知：已触页当场翻叶子 PTE，未触页仅同步簿记（Map.flags）。
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let flags = PteFlags::from_bits_truncate(frame.gpr.x(Gprs::A2) as u64);
            let ok = ident.team.space.mprotect(addr, size, flags).is_ok();
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
    };
    frame as *mut TrapContext
}
