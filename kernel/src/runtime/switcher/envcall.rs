// 环境调用（envcall）— 用户态经 ecall 请求内核执行环境服务
//
// RISC-V 特权规范：U 态 ecall 即 "Environment Call"（riscv crate 官方枚举亦名
// `Exception::UserEnvCall`）——本模块即该调用的内核侧 ABI，术语与规范同源。
//
// 方案 3（typed payload）：a7 = 调用号（slot = class << 32 | index，由
// derive(Envcall) 的 slot() 现算），a0..a5 = 参数按 `Wire` 校验式 unpack。
// 本模块不再手读 `frame.gpr.x(A0) as u64`，而是 `EnvCall::from_wire(slot, &regs)`
// 一次解码出带类型载荷的 variant，match 各 arm 直接消费类型化字段。
// `Permission` 子集在 decode 时已过 `from_bits(...).ok_or(...)` 校验（非法位 → Err），
// 根除旧 `from_bits_truncate` 的静默截断；`PteFlags` 仍在 `Mprotect` arm 校验。
// 返回值写回 a0（`Gprs::A0`）；每个调用后 sepc += 4（Reap 除外——不返回）。
// 时间语义统一以毫秒（Duration 边界）表达（Park / Wait）；Ticks 仅作兼容诊断。
// 调用名与调度词族（conductor）同词：Starve/Park/Reap/Wait/Wake 即 utask 各服务。

use core::time::Duration;

use alloc::sync::Arc;

use ubi::{
    ChronoCall, ControlCall, EnvCall, IOCall, MailCall, MemoryCall, PieToken, RoomCall, UnitCall,
};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::mail::HOLE_MSG_LEN;
use crate::work::unit::gate::{self, AnyPie, GateError, Need, Permission, Pie};
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState};
use crate::work::unit::task::TaskIdent;
use crate::work::room::messenger::WaitKey;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::utask::{park, reap, starve, wait, wake};

/// Permission 子集 → PteFlags（cap ⊆ 页表的翻译：subset 决定页表实际权限）。
///
/// | subset                  | PteFlags                |
/// |-------------------------|-------------------------|
/// | READ                    | V\|R\|U\|A\|D           |
/// | READ \| WRITE           | V\|R\|W\|U\|A\|D        |
/// | other（含空 / 仅 WRITE）| Denied                  |
fn subset_to_pte(subset: Permission) -> Result<PteFlags, GateError> {
    if !subset.contains(Permission::READ) {
        return Err(GateError::Denied);
    }
    let mut f = PteFlags::V | PteFlags::U | PteFlags::A | PteFlags::D;
    f |= PteFlags::R;
    if subset.contains(Permission::WRITE) {
        f |= PteFlags::W;
    }
    Ok(f)
}

/// 写回错误码并返回待恢复帧。
fn ret_err(frame: &mut TrapContext, e: GateError) -> *mut TrapContext {
    frame.gpr.set_x(Gprs::A0, e.code() as usize);
    frame as *mut TrapContext
}

/// 从 pie 句柄取 u64 token。
fn tok(t: PieToken) -> u64 {
    t.get()
}

/// envcall 分发。
///
/// 入参 frame = 当前任务用户帧；`ident` = 当前任务身份（**Arc 所有权移交**——
/// 可能触发 halt 的分支（Reap/Park/Wait → run）须先 `drop(ident)`，否则 halt
/// 时身份 Arc 仍持最后任务 team → space 不 drop，关机审计误报帧泄漏）。
/// 返回待恢复帧：Starve/Park 返回下一任务帧，Reap 返回后调用方不得再触碰 frame。
pub fn dispatch(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> *mut TrapContext {
    let number = frame.gpr.x(Gprs::A7);
    let regs = [
        frame.gpr.x(Gprs::A0),
        frame.gpr.x(Gprs::A1),
        frame.gpr.x(Gprs::A2),
        frame.gpr.x(Gprs::A3),
        frame.gpr.x(Gprs::A4),
        frame.gpr.x(Gprs::A5),
    ];
    let envcall = match EnvCall::from_wire(number, &regs) {
        Ok(c) => c,
        Err(_) => panic!("invalid envcall number: {number}"),
    };
    trace::note(EventKind::Env(EnvEvent::Call {
        call: number,
        arg: frame.gpr.x(Gprs::A0),
    }));
    frame.sepc += 4;
    match envcall {
        EnvCall::Room(RoomCall::Starve) => return starve() as *mut TrapContext,
        EnvCall::IO(IOCall::Put { len, buf }) => {
            let ok = crate::console::push(&ident.team.space, buf.get(), len);
            if !ok {
                frame.gpr.set_x(Gprs::A0, usize::MAX);
            }
        }
        EnvCall::IO(IOCall::Get) => {
            frame.gpr.set_x(
                Gprs::A0,
                match crate::console::pull() {
                    Some(b) => b as usize,
                    None => -2isize as usize,
                },
            );
        }
        EnvCall::Room(RoomCall::Reap) => {
            drop(ident);
            return reap() as *mut TrapContext;
        }
        EnvCall::Chrono(ChronoCall::Ticks) => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        EnvCall::Room(RoomCall::Park { millis }) => {
            drop(ident);
            return park(Duration::from_millis(millis as u64)) as *mut TrapContext;
        }
        EnvCall::Room(RoomCall::Wait { key, millis }) => {
            let wkey = WaitKey::compose(ident.team.space.asid(), key);
            let dur = if millis == usize::MAX {
                Duration::MAX
            } else {
                Duration::from_millis(millis as u64)
            };
            drop(ident);
            if let Some(pa) = wait(wkey, dur) {
                return pa as *mut TrapContext;
            }
        }
        EnvCall::Room(RoomCall::Wake { key }) => {
            let wkey = WaitKey::compose(ident.team.space.asid(), key);
            let woke = wake(wkey);
            frame.gpr.set_x(Gprs::A0, woke as usize);
        }
        EnvCall::Chrono(ChronoCall::Clock) => {
            let up = clock::uptime();
            frame.gpr.set_x(Gprs::A0, up.as_secs() as usize);
            frame.gpr.set_x(Gprs::A1, up.subsec_nanos() as usize);
        }
        EnvCall::Memory(MemoryCall::Allocate { size }) => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
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
        EnvCall::Memory(MemoryCall::Deallocate { addr, size }) => {
            let addr = addr.get();
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let ok = {
                let s = &ident.team.space;
                let freed = HeapWindow::deallocate(s, KVirt::from_raw(addr), size);
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
        EnvCall::Unit(UnitCall::Spawn { entry, arg, stack }) => {
            let entry = KVirt::from_raw(entry);
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
        EnvCall::Unit(UnitCall::SelfId) => {
            let id = current()
                .running_task()
                .map(|t| t.ident.id)
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
        }
        EnvCall::Unit(UnitCall::SpawnTeam { which }) => {
            // 装载镜像成独立域（建 Space+Team，不产 task）。血缘：当前运行 task 为 sire。
            let sire = current()
                .running_task()
                .map(|t| alloc::sync::Arc::downgrade(&t))
                .unwrap_or_default();
            let r = crate::work::unit::domain::spawn_team(which, sire);
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(team) => team.id.get(),
                    Err(_) => usize::MAX,
                },
            );
        }
        EnvCall::Unit(UnitCall::SpawnTask { team, entry, arg }) => {
            // 在指定 team 下建线程（域内产 task）。
            let team_arc = crate::work::unit::team::lookup_team(team);
            let team_arc = match team_arc {
                Some(t) => t,
                None => {
                    frame.gpr.set_x(Gprs::A0, usize::MAX);
                    return frame as *mut TrapContext;
                }
            };
            // entry=0 用域默认入口（spawn_team 装载的镜像 e_entry）；否则用户指定。
            let entry = if entry == 0 { team_arc.default_entry() } else { entry };
            let r = team_arc.task().name("u-thread").entry(KVirt::from_raw(entry)).arg(arg).spawn();
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(id) => id,
                    Err(_) => usize::MAX,
                },
            );
        }
        EnvCall::Control(ControlCall::Panic { code }) => {
            panic!("user-initiated panic (code {code:#x})");
        }
        EnvCall::Memory(MemoryCall::Mmap { size, at }) => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let fixed = at.get();
            let va = {
                let s = &ident.team.space;
                if fixed == 0 {
                    ShareWindow::mmap(s, size).map(|span| span.va)
                } else {
                    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U;
                    s.map(KVirt::from_raw(fixed), size, flags, Some(Pending::Lazy))
                        .map(|()| KVirt::from_raw(fixed))
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
        EnvCall::Memory(MemoryCall::Munmap { addr, size }) => {
            let addr = KVirt::from_raw(addr.get());
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
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
        EnvCall::Memory(MemoryCall::Mprotect { addr, size, flags }) => {
            let addr = KVirt::from_raw(addr.get());
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            // 校验式：非法位 → 拒绝（不再 from_bits_truncate 静默截断）。
            let ok = match PteFlags::from_bits(flags) {
                Some(f) => ident.team.space.protect(addr, size, f).is_ok(),
                None => false,
            };
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        EnvCall::Mail(MailCall::UnsealHole) => {
            // 编排创建：mail::hole::meta() 建实体 → gate::new_pie 建门闩 → 落 pies。
            // meta() 只建 HoleMeta + 注册 memo；门闩/落 task.pies 是能力模型的事。
            let r = (|| -> Result<u64, GateError> {
                let task = current().running_task().ok_or(GateError::Denied)?;
                let (meta, id) = mail::hole::meta()?;
                let pie: Pie<mail::hole::HoleMeta> = gate::new_pie(
                    id,
                    Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
                    None, // 原始自持：无 vestor
                    alloc::sync::Arc::downgrade(&meta),
                );
                let token = pie.token();
                task.pies.lock().push(AnyPie::Hole(pie));
                Ok(token)
            })();
            match r {
                Ok(token) => frame.gpr.set_x(Gprs::A0, token as usize),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        EnvCall::Mail(MailCall::UnsealPole { bytes }) => {
            // 编排创建：mail::pole::meta() 建实体 → gate::new_pie 建门闩 → 落 pies →
            // auto-map 创建者视图（创建者 pie 全权 → R|W）。
            let r = (|| -> Result<u64, GateError> {
                let task = current().running_task().ok_or(GateError::Denied)?;
                let (meta, id) = mail::pole::meta(bytes)?;
                let task_space = task.ident.team.space.clone();
                let pie: Pie<mail::pole::PoleMeta> = gate::new_pie(
                    id,
                    Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
                    None, // 原始自持：无 vestor
                    alloc::sync::Arc::downgrade(&meta),
                );
                let token = pie.token();
                // 创建者自留 pie 全权 → map 走 R|W。
                let creator_flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U
                    | PteFlags::A | PteFlags::D;
                mail::pole::map(&meta, token, &task_space, creator_flags)?;
                task.pies.lock().push(AnyPie::Pole(pie));
                Ok(token)
            })();
            match r {
                Ok(token) => frame.gpr.set_x(Gprs::A0, token as usize),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        EnvCall::Mail(MailCall::Push { token, msg }) => {
            let token = tok(token);
            let va = msg.get();
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.allows(Need::Write) {
                    return Some(Err(GateError::Denied));
                }
                if !pie.alive() {
                    return Some(Err(GateError::Dead));
                }
                match pie {
                    AnyPie::Hole(p) => p.weak.upgrade().map(|a| Ok(a)),
                    _ => None,
                }
            }) {
                Some(Ok(meta)) => {
                    let mut msg = [0u8; HOLE_MSG_LEN];
                    if !mail::copy_in(&ident.team.space, &mut msg, va) {
                        Err(GateError::Denied)
                    } else {
                        mail::hole::push(&meta, &msg)
                    }
                }
                Some(Err(e)) => Err(e),
                None => Err(GateError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Pull { token, buf }) => {
            let token = tok(token);
            let va = buf.get();
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.allows(Need::Read) {
                    return Some(Err(GateError::Denied));
                }
                if !pie.alive() {
                    return Some(Err(GateError::Dead));
                }
                match pie {
                    AnyPie::Hole(p) => p.weak.upgrade().map(|a| Ok(a)),
                    _ => None,
                }
            }) {
                Some(Ok(meta)) => match mail::hole::pull(&meta) {
                    Ok(m) => {
                        if !mail::copy_out(&ident.team.space, &m, va) {
                            Err(GateError::Denied)
                        } else {
                            Ok(())
                        }
                    }
                    Err(e) => Err(e),
                },
                Some(Err(e)) => Err(e),
                None => Err(GateError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Map { token }) => {
            let token = tok(token);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.allows(Need::Read) {
                    return Some(Err(GateError::Denied));
                }
                if !pie.alive() {
                    return Some(Err(GateError::Dead));
                }
                match pie {
                    AnyPie::Pole(p) => {
                        let flags = subset_to_pte(pie.permission());
                        p.weak.upgrade().map(|a| Ok((a, token, flags)))
                    }
                    _ => None,
                }
            }) {
                Some(Ok((meta, token, Ok(flags)))) => {
                    mail::pole::map(&meta, token, &ident.team.space, flags)
                }
                Some(Ok((_, _, Err(e)))) => Err(e),
                Some(Err(e)) => Err(e),
                None => Err(GateError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(v) => v,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Unmap { token }) => {
            let token = tok(token);
            let task = current().running_task();
            let r = match task.and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.allows(Need::Read) {
                    return Some(Err(GateError::Denied));
                }
                if !pie.alive() {
                    return Some(Err(GateError::Dead));
                }
                match pie {
                    AnyPie::Pole(p) => p.weak.upgrade().map(|a| Ok((a, token))),
                    _ => None,
                }
            }) {
                Some(Ok((meta, token))) => mail::pole::unmap(&meta, token),
                Some(Err(e)) => Err(e),
                None => Err(GateError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Seal { token }) => {
            let token = tok(token);
            let task = current().running_task();
            let resource = match task.as_ref().and_then(|t| {
                let pies = t.pies.lock();
                pies.iter()
                    .find(|p| p.token() == token)
                    .map(|p| p.resource())
            }) {
                Some(r) => r,
                None => return ret_err(frame, GateError::Denied),
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
                None => Err(GateError::Dead),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(()) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Accord { src, dst, subset }) => {
            let src_token = tok(src);
            let dst_id = dst.get();

            let src_task = current().running_task();
            let current_id = src_task.as_ref().map(|t| t.ident.id).unwrap_or(0);
            let src = match src_task
                .and_then(|t| t.pies.lock().iter().find(|p| p.token() == src_token).cloned())
            {
                Some(p) => p,
                None => return ret_err(frame, GateError::Denied),
            };
            if !src.alive() {
                return ret_err(frame, GateError::Dead);
            }
            if !src.allows(Need::Grant) {
                return ret_err(frame, GateError::Denied);
            }
            // subset 已由 Wire 校验式 unpack（非法位 → Err），此处仅查非空 & ⊆ 当前权限。
            if !src.covers(subset) {
                return ret_err(frame, GateError::Denied);
            }
            // BACK 守门：带 BACK 只能回授 vestor（原始自持 None 不受限）。
            if !src.vestable_to(dst_id) {
                return ret_err(frame, GateError::Denied);
            }
            let target =
                match crate::work::room::scheduler::core::lookup_task_by_id_weak(dst_id) {
                    Some(w) => w,
                    None => return ret_err(frame, GateError::Denied),
                };
            let r = gate::accord(&src, &target, subset, current_id);
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(token) => token as usize,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Narrow { token, subset }) => {
            let token = tok(token);
            let task = current().running_task();
            let meta = match task.as_ref().and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.covers(subset) {
                    return Some(Err(GateError::Denied));
                }
                match pie {
                    AnyPie::Hole(p) => {
                        if !p.alive() {
                            return Some(Err(GateError::Dead));
                        }
                        Some(Ok(None))
                    }
                    AnyPie::Pole(p) => match p.weak.upgrade() {
                        Some(arc) => Some(Ok(Some(arc))),
                        None => Some(Err(GateError::Dead)),
                    },
                }
            }) {
                None => Err(GateError::Denied),
                Some(Err(e)) => Err(e),
                Some(Ok(v)) => Ok(v),
            };
            let meta = match meta {
                Ok(m) => m,
                Err(e) => return ret_err(frame, e),
            };
            if let Some(meta) = meta {
                let flags = match subset_to_pte(subset) {
                    Ok(f) => f,
                    Err(e) => return ret_err(frame, e),
                };
                if let Err(e) = mail::pole::narrow(&meta, token, flags) {
                    return ret_err(frame, e);
                }
            }
            let ok = task.and_then(|t| {
                let mut pies = t.pies.lock();
                pies.iter_mut()
                    .find(|p| p.token() == token)
                    .map(|p| gate::narrow(p, subset))
            });
            frame.gpr.set_x(
                Gprs::A0,
                match ok {
                    Some(Ok(())) => 0,
                    Some(Err(e)) => e.code() as usize,
                    None => GateError::Denied.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Revoke { dst, token }) => {
            let dst_id = dst.get();
            let token = tok(token);
            let current_id = current().running_task().map(|t| t.ident.id).unwrap_or(0);
            let target =
                match crate::work::room::scheduler::core::lookup_task_by_id_weak(dst_id) {
                    Some(w) => w,
                    None => return ret_err(frame, GateError::Denied),
                };
            let r = gate::revoke(&target, token, current_id);
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
