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

use ubi::{
    ChronoCall, ControlCall, IOCall, MailCall, MemoryCall, RoomCall, TaskCall, Ucall,
};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::conductor::core::WaitKey;
use crate::work::room::conductor::utask::{park, reap, starve, wait, wake};
use crate::work::mail::port::{self, MSG_LEN, PortError};
use crate::work::mail::{copy_in, copy_out};
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
        Ucall::Room(RoomCall::Starve) => return starve() as *mut TrapContext,
        Ucall::IO(IOCall::Put) => {
            // 以后接入 operator: file system adaptor
            let len = frame.gpr.x(Gprs::A0);
            let ptr = frame.gpr.x(Gprs::A1);
            let ok = crate::console::write_in(&ident.team.space, ptr, len);
            if !ok {
                frame.gpr.set_x(Gprs::A0, usize::MAX);
            }
        }
        Ucall::Room(RoomCall::Reap) => {
            return reap() as *mut TrapContext;
        }
        Ucall::Chrono(ChronoCall::Ticks) => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        Ucall::Room(RoomCall::Park) => {
            // sepc 前进（唤醒恢复时从 ecall 之后继续）。
            // a0 = 毫秒（Duration 边界；换算按 timebase 进行）
            return park(Duration::from_millis(frame.gpr.x(Gprs::A0) as u64)) as *mut TrapContext;
        }
        Ucall::Room(RoomCall::Wait) => {
            // a0 = key（用户空间事件地址），a1 = 毫秒（usize::MAX = 永久）。
            // key 合成空间身份（asid 高 16 位 || va 低 48 位）——跨空间同 VA 不得
            // 混淆（与 fence::key 同源意；WaitKey::compose 单射要求 va < 2^48）。
            let key = WaitKey::compose(
                ident.team.space.asid() as usize,
                frame.gpr.x(Gprs::A0),
            );
            let ms = frame.gpr.x(Gprs::A1);
            let dur = if ms == usize::MAX {
                Duration::MAX
            } else {
                Duration::from_millis(ms as u64)
            };
            return wait(key, dur) as *mut TrapContext;
        }
        Ucall::Room(RoomCall::Wake) => {
            // a0 = key（与 Wait 同源合成）。
            let key = WaitKey::compose(
                ident.team.space.asid() as usize,
                frame.gpr.x(Gprs::A0),
            );
            let woke = wake(key);
            frame.gpr.set_x(Gprs::A0, woke as usize);
        }
        Ucall::Chrono(ChronoCall::Clock) => {
            let up = clock::uptime();
            frame.gpr.set_x(Gprs::A0, up.as_secs() as usize);
            frame.gpr.set_x(Gprs::A1, up.subsec_nanos() as usize);
        }
        Ucall::Memory(MemoryCall::Allocate) => {
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
        Ucall::Memory(MemoryCall::Deallocate) => {
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
        Ucall::Task(TaskCall::Spawn) => {
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
        Ucall::Control(ControlCall::Panic) => {
            // 用户主动 panic：a0 = 呼叫人指定的关联码。
            panic!("user-initiated panic (code {:#x})", frame.gpr.x(Gprs::A0));
        }
        Ucall::Memory(MemoryCall::Mmap) => {
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
        Ucall::Memory(MemoryCall::Munmap) => {
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
        Ucall::Memory(MemoryCall::Mprotect) => {
            // a0 = 映射 VA，a1 = 字节数（页对齐），a2 = 新权限（PteFlags 位）。
            // 懒区感知：已触页当场翻叶子 PTE，未触页仅同步簿记（Map.flags）。
            let addr = VirtAddr::from_raw(frame.gpr.x(Gprs::A0));
            let size = frame.gpr.x(Gprs::A1).max(1).next_multiple_of(PAGE_SIZE);
            let flags = PteFlags::from_bits_truncate(frame.gpr.x(Gprs::A2) as u64);
            let ok = ident.team.space.mprotect(addr, size, flags).is_ok();
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Mail(MailCall::PortOpen) => {
            // 建 port（内核邮路）：a0 = 句柄，a1 = 条件键（用户侧 wait/wake 用）。
            let (handle, key) = port::open();
            frame.gpr.set_x(Gprs::A0, handle);
            frame.gpr.set_x(Gprs::A1, key);
        }
        Ucall::Mail(MailCall::PortShut) => {
            // 终止 port：置 Dead（对端 push/pull 感知断开）。a0 = 句柄。
            let ok = port::shut(frame.gpr.x(Gprs::A0));
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        Ucall::Mail(MailCall::PortPush) => {
            // a0 = 句柄，a1 = 消息 VA（定长 [MSG_LEN] 字节拷贝进内核槽）。
            // 返回：0 = 存入；负码 -2 = 槽满（Busy，调用方 wait 后重试）；-1 = Dead。
            let handle = frame.gpr.x(Gprs::A0);
            let va = frame.gpr.x(Gprs::A1);
            let mut msg = [0u8; MSG_LEN];
            if !copy_in(&ident.team.space, &mut msg, va) {
                frame.gpr.set_x(Gprs::A0, usize::MAX); // 缓冲不可读（D1 负值）
            } else {
                frame.gpr.set_x(
                    Gprs::A0,
                    match port::try_push(handle, &msg) {
                        Ok(()) => 0,
                        Err(PortError::Busy) => -2isize as usize,
                        Err(PortError::Dead) => -1isize as usize,
                    },
                );
            }
        }
        Ucall::Mail(MailCall::PortPull) => {
            // a0 = 句柄，a1 = 缓冲 VA（槽内消息拷贝出内核）。
            // 返回：0 = 取出；-2 = 槽空（Busy，wait 后重试）；-1 = Dead。
            let handle = frame.gpr.x(Gprs::A0);
            let va = frame.gpr.x(Gprs::A1);
            match port::try_pull(handle) {
                Ok(msg) => {
                    if copy_out(&ident.team.space, &msg, va) {
                        frame.gpr.set_x(Gprs::A0, 0);
                    } else {
                        frame.gpr.set_x(Gprs::A0, usize::MAX); // 缓冲不可写
                    }
                }
                Err(PortError::Busy) => frame.gpr.set_x(Gprs::A0, -2isize as usize),
                Err(PortError::Dead) => frame.gpr.set_x(Gprs::A0, -1isize as usize),
            }
        }
        Ucall::Mail(MailCall::RingOpen) | Ucall::Mail(MailCall::RingShut) => {
            // ring 用户侧设施缓行（共享区布局待真实消费方再设计）；槽位已占，
            // 首版不可达。
            panic!("mail ring user-side not wired (缓行)");
        }
    };
    frame as *mut TrapContext
}
