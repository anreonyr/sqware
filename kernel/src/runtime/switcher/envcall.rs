// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 约定：a7 = 调用号（slot = 前一半 usize 功能分类 || 后一半序号，见 ubi::Ucall），
// a0..a5 = 参数，返回值写回 a0/a1（Gprs::A0/A1）；每个调用后 sepc += 4（Reap
// 除外——不返回）。时间语义统一以毫秒（Duration 边界）表达（Park / Wait）；
// Ticks 仅作兼容诊断，非时间单位。调用名与调度词族（conductor）同词：
// Starve/Park/Reap/Wait/Wake 即 utask 各服务。

use core::time::Duration;

use alloc::sync::Arc;

use ubi::{ChronoCall, ControlCall, IOCall, MailCall, MemoryCall, RoomCall, TaskCall, Ucall};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::mail::{HOLE_MSG_LEN, MailError, Permission, Pie, PieKind};
use crate::work::room::messenger::WaitKey;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::utask::{park, reap, starve, wait, wake};
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState};
use crate::work::unit::task::TaskIdent;

/// envcall 分发。
///
/// 入参 frame = 当前任务用户帧；`ident` = 当前任务身份（**Arc 所有权移交**——
/// 可能触发 halt 的分支（Reap/Park/Wait → run）须先 `drop(ident)`，否则 halt
/// 时身份 Arc 仍持最后任务 team → space 不 drop，关机审计误报帧泄漏）。
/// 返回待恢复帧：Starve/Park 返回下一任务帧，Reap 返回后调用方不得再触碰 frame。
pub fn dispatch(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> *mut TrapContext {
    let number = frame.gpr.x(Gprs::A7);
    let call =
        Ucall::try_from(number).unwrap_or_else(|_| panic!("invalid envcall number: {number}"));
    trace::note(EventKind::Env(EnvEvent::Call {
        call: number,
        arg: frame.gpr.x(Gprs::A0),
    }));
    frame.sepc += 4;
    match call {
        Ucall::Room(RoomCall::Starve) => return starve() as *mut TrapContext,
        Ucall::IO(IOCall::Put) => {
            let len = frame.gpr.x(Gprs::A0);
            let ptr = frame.gpr.x(Gprs::A1);
            let ok = crate::console::push(&ident.team.space, ptr, len);
            if !ok {
                frame.gpr.set_x(Gprs::A0, usize::MAX);
            }
        }
        Ucall::IO(IOCall::Get) => {
            frame.gpr.set_x(
                Gprs::A0,
                match crate::console::pull() {
                    Some(b) => b as usize,
                    None => -2isize as usize,
                },
            );
        }
        Ucall::Room(RoomCall::Reap) => {
            drop(ident);
            return reap() as *mut TrapContext;
        }
        Ucall::Chrono(ChronoCall::Ticks) => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        Ucall::Room(RoomCall::Park) => {
            drop(ident);
            return park(Duration::from_millis(frame.gpr.x(Gprs::A0) as u64)) as *mut TrapContext;
        }
        Ucall::Room(RoomCall::Wait) => {
            let raw = frame.gpr.x(Gprs::A0);
            let key = WaitKey::compose(ident.team.space.asid(), raw);
            let ms = frame.gpr.x(Gprs::A1);
            let dur = if ms == usize::MAX {
                Duration::MAX
            } else {
                Duration::from_millis(ms as u64)
            };
            drop(ident);
            if let Some(pa) = wait(key, dur) {
                return pa as *mut TrapContext;
            }
        }
        Ucall::Room(RoomCall::Wake) => {
            let raw = frame.gpr.x(Gprs::A0);
            let key = WaitKey::compose(ident.team.space.asid(), raw);
            let woke = wake(key);
            frame.gpr.set_x(Gprs::A0, woke as usize);
        }
        Ucall::Chrono(ChronoCall::Clock) => {
            let up = clock::uptime();
            frame.gpr.set_x(Gprs::A0, up.as_secs() as usize);
            frame.gpr.set_x(Gprs::A1, up.subsec_nanos() as usize);
        }
        Ucall::Memory(MemoryCall::Allocate) => {
            let size = frame.gpr.x(Gprs::A0).max(1).next_multiple_of(PAGE_SIZE);
            let addr = {
                let s = &ident.team.space;
                let r = HeapWindow::allocate(s, size).map(|span| span.va);
                if let Ok(va) = r {
                    let key = crate::memory::allocator::fence::key(s.asid(), va.as_usize());
                    crate::memory::allocator::fence::on_alloc(
                        key,
                        size,
                        crate::memory::allocator::fence::OwnerKind::UserHeap,
                    );
                    #[cfg(feature = "audit")]
                    crate::memory::allocator::fence::tag(
                        key,
                        crate::memory::allocator::fence::Class::Task,
                    );
                }
                r
            };
            frame.gpr.set_x(
                Gprs::A0,
                match addr {
                    Ok(va) => va.as_usize(),
                    Err(_) => usize::MAX,
                },
            );
        }
        Ucall::Memory(MemoryCall::Deallocate) => {
            let addr = frame.gpr.x(Gprs::A0);
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                let freed = HeapWindow::deallocate(s, VirtAddr::from_raw(addr), size);
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
        Ucall::Task(TaskCall::Spawn) => {
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
                    Ok(id) => id,
                    Err(_) => usize::MAX,
                },
            );
        }
        Ucall::Control(ControlCall::Panic) => {
            panic!("user-initiated panic (code {:#x})", frame.gpr.x(Gprs::A0));
        }
        Ucall::Memory(MemoryCall::Mmap) => {
            let size = frame.gpr.x(Gprs::A0).max(1).next_multiple_of(PAGE_SIZE);
            let fixed = frame.gpr.x(Gprs::A2);
            let va = {
                let s = &ident.team.space;
                if fixed == 0 {
                    ShareWindow::mmap(s, size).map(|span| span.va)
                } else {
                    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
                    s.map(VirtAddr::from_raw(fixed), size, flags, Some(Pending::Lazy))
                        .map(|()| VirtAddr::from_raw(fixed))
                }
            };
            frame.gpr.set_x(
                Gprs::A0,
                match va {
                    Ok(va) => va.as_usize(),
                    Err(_) => usize::MAX,
                },
            );
        }
        Ucall::Memory(MemoryCall::Munmap) => {
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                if ShareWindow::munmap(s, addr, size) {
                    true
                } else if s.pending_state(addr) != PendingState::Absent {
                    s.unmap(addr, size);
                    true
                } else {
                    false
                }
            };
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Memory(MemoryCall::Mprotect) => {
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let flags = PteFlags::from_bits_truncate(frame.gpr.x(Gprs::A2) as u64);
            let ok = ident.team.space.protect(addr, size, flags).is_ok();
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Mail(MailCall::OpenHole) => {
            match mail::hole::hole_create() {
                Ok(idx) => frame.gpr.set_x(Gprs::A0, idx),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        Ucall::Mail(MailCall::OpenPole) => {
            let bytes = frame.gpr.x(Gprs::A0);
            match mail::pole::pole_create(&ident.team.space, bytes) {
                Ok(idx) => frame.gpr.set_x(Gprs::A0, idx),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        Ucall::Mail(MailCall::Push) => {
            let idx = frame.gpr.x(Gprs::A0);
            let va = frame.gpr.x(Gprs::A1);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.get(idx)?;
                if pie.kind() != PieKind::Hole { return None; }
                if !pie.permission().contains(Permission::WRITE) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                let arc = match pie {
                    mail::AnyPie::Hole(p) => p.weak.upgrade(),
                    _ => return None,
                };
                arc.map(|a| Ok(a))
            }) {
                Some(Ok(meta)) => {
                    let mut msg = [0u8; HOLE_MSG_LEN];
                    if !mail::copy_in(&ident.team.space, &mut msg, va) {
                        Err(MailError::Denied)
                    } else {
                        mail::hole::hole_push(&meta, &msg)
                    }
                }
                Some(Err(e)) => Err(e),
                None => Err(MailError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Pull) => {
            let idx = frame.gpr.x(Gprs::A0);
            let va = frame.gpr.x(Gprs::A1);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.get(idx)?;
                if pie.kind() != PieKind::Hole { return None; }
                if !pie.permission().contains(Permission::READ) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                let arc = match pie {
                    mail::AnyPie::Hole(p) => p.weak.upgrade(),
                    _ => return None,
                };
                arc.map(|a| Ok(a))
            }) {
                Some(Ok(meta)) => match mail::hole::hole_pull(&meta) {
                    Ok(m) => {
                        if !mail::copy_out(&ident.team.space, &m, va) {
                            Err(MailError::Denied)
                        } else {
                            Ok(())
                        }
                    }
                    Err(e) => Err(e),
                }
                Some(Err(e)) => Err(e),
                None => Err(MailError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Map) => {
            let idx = frame.gpr.x(Gprs::A0);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.get(idx)?;
                if pie.kind() != PieKind::Pole { return None; }
                if !pie.permission().contains(Permission::READ | Permission::WRITE) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                let arc = match pie {
                    mail::AnyPie::Pole(p) => p.weak.upgrade(),
                    _ => return None,
                };
                arc.map(|a| Ok(a))
            }) {
                Some(Ok(meta)) => mail::pole::pole_map(&meta, &ident.team.space),
                Some(Err(e)) => Err(e),
                None => Err(MailError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(v) => v,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Unmap) => {
            let idx = frame.gpr.x(Gprs::A0);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.get(idx)?;
                if pie.kind() != PieKind::Pole { return None; }
                if !pie.permission().contains(Permission::READ | Permission::WRITE) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                let arc = match pie {
                    mail::AnyPie::Pole(p) => p.weak.upgrade(),
                    _ => return None,
                };
                arc.map(|a| Ok(a))
            }) {
                Some(Ok(meta)) => mail::pole::pole_unmap(&meta, &ident.team.space),
                Some(Err(e)) => Err(e),
                None => Err(MailError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Shut) => {
            let idx = frame.gpr.x(Gprs::A0);
            let task = current().running_task();
            // 取 (kind, resource)：kind 决定 shut 分派；resource 给资源表查 Meta。
            let (kind, resource) = match task.as_ref()
                .and_then(|t| {
                    let pies = t.pies.lock();
                    let pie = pies.get(idx)?;
                    if pie.permission().is_empty() { return Some(Err(MailError::Denied)); }
                    if !pie.alive() { return Some(Err(MailError::Dead)); }
                    Some(Ok((pie.kind(), pie.resource())))
                })
                .transpose()
            {
                Ok(Some((k, r))) => (k, r),
                Ok(None) => (PieKind::Hole, mail::ResourceId(0)), // 不会到 Shut 分派
                Err(e) => {
                    frame.gpr.set_x(Gprs::A0, e.code() as usize);
                    return frame as *mut TrapContext;
                }
            };
            let r = match kind {
                PieKind::Hole => {
                    if let Some(meta) = mail::resource_table::lookup_hole(resource) {
                        mail::hole::hole_shut(&meta, resource);
                        Ok(())
                    } else {
                        Err(MailError::Dead)
                    }
                }
                PieKind::Pole => {
                    if let Some(meta) = mail::resource_table::lookup_pole(resource) {
                        mail::pole::pole_shut(&meta, resource);
                        Ok(())
                    } else {
                        Err(MailError::Dead)
                    }
                }
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Vest) => {
            // a0 = src_pie_idx, a1 = target_task_id, a2 = subset bits。
            let src_idx = frame.gpr.x(Gprs::A0);
            let target_id = frame.gpr.x(Gprs::A1);
            let subset_bits = frame.gpr.x(Gprs::A2) as u32;

            let src_task = current().running_task();
            // 1. 取 src pie + 鉴权（VEST 权、subset 合法、alive）
            let src = match src_task.and_then(|t| t.pies.lock().get(src_idx).cloned()) {
                Some(p) => p,
                None => {
                    frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                    return frame as *mut TrapContext;
                }
            };
            if !src.alive() {
                frame.gpr.set_x(Gprs::A0, MailError::Dead.code() as usize);
                return frame as *mut TrapContext;
            }
            if !src.permission().contains(Permission::VEST) {
                frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                return frame as *mut TrapContext;
            }
            let subset = Permission::from_bits_truncate(subset_bits);
            if subset.is_empty() || (subset & src.permission()) != subset {
                frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                return frame as *mut TrapContext;
            }
            // 2. 查 target task
            let target = match crate::work::room::scheduler::core::lookup_task_by_id(target_id) {
                Some(t) => t,
                None => {
                    frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                    return frame as *mut TrapContext;
                }
            };
            // 3. 调 vest 数据面原语
            let r = mail::vest::vest(&src, &target, subset);
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(idx) => idx,
                    Err(e) => e.code() as usize,
                },
            );
        }
    };
    frame as *mut TrapContext
}