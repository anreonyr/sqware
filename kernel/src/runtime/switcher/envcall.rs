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
use crate::work::mail::{AnyPie, HOLE_MSG_LEN, MailError, Permission};
use crate::work::room::messenger::WaitKey;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::utask::{park, reap, starve, wait, wake};
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState};
use crate::work::unit::task::TaskIdent;

/// Permission 子集 → PteFlags（cap ⊆ 页表的翻译：subset 决定页表实际权限）。
///
/// | subset                  | PteFlags                |
/// |-------------------------|-------------------------|
/// | READ                    | V\|R\|U\|A\|D           |
/// | READ \| WRITE           | V\|R\|W\|U\|A\|D        |
/// | other（含空 / 仅 WRITE）| Denied                  |
fn subset_to_pte(subset: Permission) -> Result<PteFlags, MailError> {
    if !subset.contains(Permission::READ) {
        return Err(MailError::Denied);
    }
    let mut f = PteFlags::V | PteFlags::U | PteFlags::A | PteFlags::D;
    f |= PteFlags::R;
    if subset.contains(Permission::WRITE) {
        f |= PteFlags::W;
    }
    Ok(f)
}

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
        Ucall::Task(TaskCall::SelfId) => {
            // 返当前 task id（无参；demo 用作"已知非-vestor" 或与共享 vestor 槽配对）。
            let id = current()
                .running_task()
                .map(|t| t.ident.id)
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
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
        Ucall::Mail(MailCall::UnsealHole) => {
            match mail::hole::unseal() {
                Ok(token) => frame.gpr.set_x(Gprs::A0, token as usize),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        Ucall::Mail(MailCall::UnsealPole) => {
            let bytes = frame.gpr.x(Gprs::A0);
            match mail::pole::unseal(bytes) {
                Ok(token) => frame.gpr.set_x(Gprs::A0, token as usize),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        Ucall::Mail(MailCall::Push) => {
            let token = frame.gpr.x(Gprs::A0) as u64;
            let va = frame.gpr.x(Gprs::A1);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.permission().contains(Permission::WRITE) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                match pie {
                    AnyPie::Hole(p) => p.weak.upgrade().map(|a| Ok(a)),
                    _ => None,
                }
            }) {
                Some(Ok(meta)) => {
                    let mut msg = [0u8; HOLE_MSG_LEN];
                    if !mail::copy_in(&ident.team.space, &mut msg, va) {
                        Err(MailError::Denied)
                    } else {
                        mail::hole::push(&meta, &msg)
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
            let token = frame.gpr.x(Gprs::A0) as u64;
            let va = frame.gpr.x(Gprs::A1);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.permission().contains(Permission::READ) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                match pie {
                    AnyPie::Hole(p) => p.weak.upgrade().map(|a| Ok(a)),
                    _ => None,
                }
            }) {
                Some(Ok(meta)) => match mail::hole::pull(&meta) {
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
            let token = frame.gpr.x(Gprs::A0) as u64;
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.permission().contains(Permission::READ) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                match pie {
                    AnyPie::Pole(p) => {
                        // cap ⊆ 页表：subset 决定 flags（READ→R，READ|WRITE→R|W）
                        let flags = subset_to_pte(pie.permission());
                        p.weak.upgrade().map(|a| Ok((a, token, flags)))
                    }
                    _ => None,
                }
            }) {
                Some(Ok((meta, token, Ok(flags)))) => mail::pole::map(&meta, token, &ident.team.space, flags),
                Some(Ok((_, _, Err(e)))) => Err(e),
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
            let token = frame.gpr.x(Gprs::A0) as u64;
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.permission().contains(Permission::READ) { return Some(Err(MailError::Denied)); }
                if !pie.alive() { return Some(Err(MailError::Dead)); }
                match pie {
                    AnyPie::Pole(p) => p.weak.upgrade().map(|a| Ok((a, token))),
                    _ => None,
                }
            }) {
                Some(Ok((meta, token))) => mail::pole::unmap(&meta, token),
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
        Ucall::Mail(MailCall::Seal) => {
            let token = frame.gpr.x(Gprs::A0) as u64;
            let task = current().running_task();
            // 取 resource：token 定位自己 pie。seal 免费（任何持有者皆可封印）。
            let resource = match task.as_ref().and_then(|t| {
                let pies = t.pies.lock();
                pies.iter().find(|p| p.token() == token).map(|p| p.resource())
            }) {
                Some(r) => r,
                None => {
                    frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                    return frame as *mut TrapContext;
                }
            };
            let r = match mail::memo::lookup(resource) {
                Some(mail::memo::Meta::Hole(m)) => {
                    mail::hole::seal(&m, resource);
                    Ok(())
                }
                Some(mail::memo::Meta::Pole(m)) => {
                    mail::pole::seal(&m, resource);
                    Ok(())
                }
                None => Err(MailError::Dead),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Accord) => {
            // a0 = src_token, a1 = dst_id, a2 = subset bits。
            let src_token = frame.gpr.x(Gprs::A0) as u64;
            let dst_id = frame.gpr.x(Gprs::A1);
            let subset_bits = frame.gpr.x(Gprs::A2) as u32;

            let src_task = current().running_task();
            // 提前取 id（src_task 在 and_then 闭包里被 move 消费）
            let current_id = src_task.as_ref().map(|t| t.ident.id).unwrap_or(0);
            // 1. 取 src pie + 鉴权（VEST 或 BACK 权、subset 合法、alive）
            let src = match src_task
                .and_then(|t| t.pies.lock().iter().find(|p| p.token() == src_token).cloned())
            {
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
            // 含 VEST 或 BACK 任一即能 accord（BACK 是受限 accord，详下）。
            if !src.permission().contains(Permission::VEST)
                && !src.permission().contains(Permission::BACK)
            {
                frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                return frame as *mut TrapContext;
            }
            let subset = Permission::from_bits_truncate(subset_bits);
            if subset.is_empty() || (subset & src.permission()) != subset {
                frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                return frame as *mut TrapContext;
            }
            // BACK 验：源 pie 有 BACK 必 dst == src.vestor。
            //   src.vestor() = None  ⇒ 原始自持（无上一手），BACK 退化为"accord 同效"。
            //   src.vestor() = Some(g) ⇒ BACK 守门 dst == g。
            if src.permission().contains(Permission::BACK) {
                if let Some(vestor) = src.vestor() {
                    if vestor != dst_id {
                        frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                        return frame as *mut TrapContext;
                    }
                }
            }
            // 2. 查 dst task（持 Weak，避开 scheduler transform 的 strong_count==1 断言）。
            let target = match crate::work::room::scheduler::core::lookup_task_by_id_weak(dst_id) {
                Some(w) => w,
                None => {
                    frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                    return frame as *mut TrapContext;
                }
            };
            // 3. 调 accord 数据面原语（传 current_id 作新 pie 的 vestor）。返新 token（撤销句柄）。
            let r = mail::accord::accord(&src, &target, subset, current_id);
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(token) => token as usize,
                    Err(e) => e.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Narrow) => {
            // a0 = token, a1 = subset bits。就地改写本 pie 权限（单调收窄）;
            // Pole 额外把该 pie 的映射段降权——cap ⊆ 页表。
            let token = frame.gpr.x(Gprs::A0) as u64;
            let subset = Permission::from_bits_truncate(frame.gpr.x(Gprs::A1) as u32);

            // Phase A：锁内校验（非空 / 单调）+ 取 Pole meta Arc。
            // 锁内只做 Arc clone，不做空间操作（锁序纪律：pids 锁不跨 space 锁）。
            let task = current().running_task();
            let meta = match task.as_ref().and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if subset.is_empty() || (subset & pie.permission()) != subset {
                    return Some(Err(MailError::Denied));
                }
                match pie {
                    AnyPie::Hole(p) => {
                        if !p.alive() { return Some(Err(MailError::Dead)); }
                        Some(Ok(None))
                    }
                    AnyPie::Pole(p) => match p.weak.upgrade() {
                        Some(arc) => Some(Ok(Some(arc))),
                        None => Some(Err(MailError::Dead)),
                    },
                }
            }) {
                None => Err(MailError::Denied),
                Some(Err(e)) => Err(e),
                Some(Ok(v)) => Ok(v),
            };
            let meta = match meta {
                Ok(m) => m,
                Err(e) => {
                    frame.gpr.set_x(Gprs::A0, e.code() as usize);
                    return frame as *mut TrapContext;
                }
            };

            // Phase B：Pole 同步降权（该 pie token 的映射段）。成功才改写。
            if let Some(meta) = meta {
                // RISC-V PTE 无 R=0,W=0 合法数据叶子 ⇒ 无 READ 的 subset 不可作为
                // Pole 降权目标。subset_to_pte 即守此门。
                let flags = match subset_to_pte(subset) {
                    Ok(f) => f,
                    Err(e) => {
                        frame.gpr.set_x(Gprs::A0, e.code() as usize);
                        return frame as *mut TrapContext;
                    }
                };
                if let Err(e) = mail::pole::narrow(&meta, token, flags) {
                    frame.gpr.set_x(Gprs::A0, e.code() as usize);
                    return frame as *mut TrapContext;
                }
            }

            // Phase C：改写 permission（数据面做单调校验 + 落值）。
            let ok = task.and_then(|t| {
                let mut pies = t.pies.lock();
                pies.iter_mut().find(|p| p.token() == token).map(|p| mail::narrow::narrow(p, subset))
            });
            frame.gpr.set_x(
                Gprs::A0,
                match ok {
                    Some(Ok(())) => 0,
                    Some(Err(e)) => e.code() as usize,
                    None => MailError::Denied.code() as usize,
                },
            );
        }
        Ucall::Mail(MailCall::Revoke) => {
            // a0 = dst_id, a1 = token。收回授与 dst 的、token 标识的副本。
            let dst_id = frame.gpr.x(Gprs::A0);
            let token = frame.gpr.x(Gprs::A1) as u64;
            let current_id = current().running_task().map(|t| t.ident.id).unwrap_or(0);

            let target = match crate::work::room::scheduler::core::lookup_task_by_id_weak(dst_id) {
                Some(w) => w,
                None => {
                    frame.gpr.set_x(Gprs::A0, MailError::Denied.code() as usize);
                    return frame as *mut TrapContext;
                }
            };
            let r = mail::revoke::revoke(&target, token, current_id);
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
    };
    frame as *mut TrapContext
}