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
use alloc::vec::Vec;

use env::{
    ChronoCall, ControlCall, EnvCall, HoleDir, IOCall, MailCall, MemoryCall, Name, RoomCall,
    UnitCall,
};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::frame::{self, ResolveCfg, StackReader};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::WaitKey;
use crate::work::room::scheduler::core::current;
use crate::work::room::scheduler::trap::run;
use crate::work::room::scheduler::utask::{self, JoinStep, park, reap, starve, wait, wake};
use crate::work::unit::gate::{self, AnyPie, GateError, Need, Permission, Pie};
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState, Space, SpaceKind};
use crate::work::unit::task::{MAX_ARGS, Task, TaskIdent};

/// 单次 `Build` 的镜像字节上限（8 MiB）：防止一次调用把内核暂存撑爆。
const MAX_IMAGE: usize = 8 * 1024 * 1024;

/// Permission 子集 → PteFlags（cap ⊆ 页表的翻译：subset 决定页表实际权限）。
///
/// | subset                  | PteFlags                |
/// |-------------------------|-------------------------|
/// | READ                    | V\|R\|A\|D              |
/// | READ \| WRITE           | V\|R\|W\|A\|D           |
/// | other（含空 / 仅 WRITE）| Denied                  |
///
/// U 位不在此处决定——由目标空间的 [`Space::pte_policy`] 加。
fn subset_to_pte(subset: Permission) -> Result<PteFlags, GateError> {
    if !subset.contains(Permission::READ) {
        return Err(GateError::Denied);
    }
    let mut f = PteFlags::V | PteFlags::A | PteFlags::D;
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

/// 陷阱指令字节数（RVC 压缩 2 字节 / 标准 4 字节）——`sepc` 前进量。
///
/// 环境调用是 `ebreak`：汇编器在开 RVC 时发 **`c.ebreak`（2 字节）**，故固定
/// `+4` 会多跳一条 2 字节指令。release 下曾因被跳过的那条恰是 `ld ra`（ra 本就
/// 未被本函数改写）而侥幸可用；debug 下跳过的是必需指令，必崩。按指令首字节低
/// 两位判长（`!= 0b11` ⇒ 2 字节）是规范做法；首字节经目标空间翻译后读，读不到
/// 按 4 字节兜底。
fn instr_len(space: &Space, sepc: KVirt) -> usize {
    let b0 = space
        .translate(sepc)
        .map(|(pa, _)| unsafe { core::ptr::read_volatile(pa.as_usize() as *const u8) })
        .unwrap_or(0b11);
    if b0 & 0b11 == 0b11 { 4 } else { 2 }
}

/// 映射错误 → 负码（`Spawn` 的栈/帧分配失败）。
fn map_err(e: crate::memory::manager::MapError) -> GateError {
    match e {
        crate::memory::manager::MapError::OutOfMemory => GateError::OoM,
        _ => GateError::Denied,
    }
}

/// 从调用方空间读一段字节（逐页翻译后拷贝；跨页安全）。
///
/// 返回 None = 长度非法 / 区间未映射（调用方按 `Denied` 处理）。一次拷进内核
/// 暂存：`Build` 的镜像与 `Spawn` 的启动参数都走这里——镜像字节只活到装载完成。
fn copy_in(space: &Space, va: KVirt, len: usize, cap: usize) -> Option<Vec<u8>> {
    if len == 0 || len > cap {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut done = 0usize;
    while done < len {
        let at = KVirt::from_raw(va.as_usize() + done);
        let (pa, _) = space.translate(at)?;
        let page_rest = PAGE_SIZE - (at.as_usize() % PAGE_SIZE);
        let n = core::cmp::min(len - done, page_rest);
        let src = pa.as_usize() as *const u8;
        for i in 0..n {
            // SAFETY: 区间已在调用方空间翻译成帧；恒等映射下 PA 可读。
            out.push(unsafe { core::ptr::read_volatile(src.add(i)) });
        }
        done += n;
    }
    Some(out)
}

/// 读调用方空间里的 `count` 个字（`Spawn` 的启动参数）。
fn copy_words(space: &Space, va: KVirt, count: usize) -> Option<Vec<usize>> {
    if count == 0 {
        return Some(Vec::new());
    }
    if count > MAX_ARGS {
        return None;
    }
    let width = size_of::<usize>();
    let bytes = copy_in(space, va, count * width, MAX_ARGS * width)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut w = [0u8; size_of::<usize>()];
        w.copy_from_slice(&bytes[i * width..(i + 1) * width]);
        out.push(usize::from_le_bytes(w));
    }
    Some(out)
}

/// 读调用方空间里的域名字（`Build` 的 name/name_len）→ 校验过的 [`Name`]。
fn read_name(space: &Space, va: KVirt, len: usize) -> Option<Name> {
    let bytes = copy_in(space, va, len, env::NAME_LEN - 1)?;
    core::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| Name::new(s).ok())
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
    trace::note(EventKind::Env(EnvEvent::Call {
        call: number,
        arg: frame.gpr.x(Gprs::A0),
    }));
    frame.sepc += instr_len(&ident.team.space, frame.sepc);
    let envcall = match EnvCall::from_wire(number, &regs) {
        Ok(c) => c,
        Err(_) => panic!("invalid envcall number: {number}"),
    };
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
            let wkey = WaitKey::compose(ident.team.space.asid().get(), key);
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
            let wkey = WaitKey::compose(ident.team.space.asid().get(), key);
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
                    let key = crate::memory::allocator::fence::key(s.asid().get(), va.as_usize());
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
                        crate::memory::allocator::fence::key(s.asid().get(), addr),
                        size,
                        crate::memory::allocator::fence::OwnerKind::UserHeap,
                    );
                }
                freed
            };
            frame.gpr.set_x(Gprs::A0, if ok { 0 } else { usize::MAX });
        }
        EnvCall::Unit(UnitCall::Spawn {
            team,
            entry,
            args,
            count,
            stack,
        }) => {
            // 目标域：TeamId(0) = 当前域；否则必须在我 heir 里（查到 = 我是 sire）
            let target = if team.get() == 0 {
                ident.team.clone()
            } else {
                match current().running_task().and_then(|me| me.heir(team)) {
                    Some(t) => t,
                    None => return ret_err(frame, GateError::Denied),
                }
            };
            // 启动参数：从调用方空间拷（count == 0 → 空）
            let words = match copy_words(&ident.team.space, KVirt::from_raw(args.get()), count) {
                Some(w) => w,
                None => return ret_err(frame, GateError::Denied),
            };
            // entry = 0 → 域默认入口（`Build` 装载所得 e_entry）
            let entry_va = if entry == 0 {
                target.default_entry()
            } else {
                entry
            };
            let mut builder = target
                .task()
                .name("u-thread")
                .entry(KVirt::from_raw(entry_va))
                .args(words);
            if stack > 0 {
                builder = builder.stack(stack);
            }
            // 恒产 Held：授权顺序由父方 `Accord` → `Hatch` 保证
            match builder.hold() {
                Ok(t) => frame.gpr.set_x(Gprs::A0, t.ident.id),
                Err(e) => return ret_err(frame, map_err(e)),
            }
        }
        EnvCall::Unit(UnitCall::SelfId) => {
            let id = current().running_task().map(|t| t.ident.id).unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
        }
        EnvCall::Unit(UnitCall::Sire) => {
            // 溯源：生我者的 task id。0 = 顶级域（boot）或父已亡。
            let id = current()
                .running_task()
                .and_then(|t| t.ident.team.sire())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
        }
        EnvCall::Unit(UnitCall::HeirCount) => {
            // 我生的子域数量（heir 枚举 first pass）。
            let n = current()
                .running_task()
                .map(|t| t.heir_count())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, n);
        }
        EnvCall::Unit(UnitCall::Heir { index }) => {
            // 按索引取子域 TeamId（heir 枚举 second pass；越界 → 0）。
            let id = current()
                .running_task()
                .and_then(|t| t.heir_at(index))
                .map(|t| t.get())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
        }
        EnvCall::Unit(UnitCall::Build {
            elf,
            len,
            kind,
            name,
            name_len,
        }) => {
            // 权限：S 态域专属（U 态建域一律拒——v1 无门控位）
            if !ident.team.space.kind().is_supervisor() {
                return ret_err(frame, GateError::Denied);
            }
            let name = match read_name(&ident.team.space, KVirt::from_raw(name.get()), name_len) {
                Some(n) => n,
                None => return ret_err(frame, GateError::Denied),
            };
            // 镜像：一次拷进内核暂存（字节只活到装载完成）
            let bytes = match copy_in(
                &ident.team.space,
                KVirt::from_raw(elf.get()),
                len,
                MAX_IMAGE,
            ) {
                Some(b) => b,
                None => return ret_err(frame, GateError::Denied),
            };
            // sire = 调用方：`build` 内部闭合血缘（域必入我 heir）
            let sire = current()
                .running_task()
                .map(|me| Arc::downgrade(&me))
                .unwrap_or_default();
            match crate::work::unit::build(&bytes, SpaceKind::from(kind), name, sire) {
                Ok(team) => frame.gpr.set_x(Gprs::A0, team.id.get()),
                Err(_) => return ret_err(frame, GateError::BadImage),
            }
        }
        EnvCall::Unit(UnitCall::Hatch { task }) => {
            let target = match crate::work::room::scheduler::core::lookup_task_by_id(task.get()) {
                Some(t) => t,
                None => return ret_err(frame, GateError::Denied),
            };
            // 授权：与我同域，或属于我 heir 里的子域
            let same = Arc::ptr_eq(&target.ident.team, &ident.team);
            let mine = current()
                .running_task()
                .map(|me| me.heir(target.ident.team.id).is_some())
                .unwrap_or(false);
            if !(same || mine) {
                return ret_err(frame, GateError::Denied);
            }
            if let Err(e) = Task::release(&target) {
                return ret_err(frame, e);
            }
        }
        EnvCall::Unit(UnitCall::Join { task, millis }) => {
            let dur = if millis == usize::MAX {
                Duration::MAX
            } else {
                Duration::from_millis(millis as u64)
            };
            // 授权：活目标须与我同域或在我 heir 里；已回收目标无从核对（返回 Dead）
            if let Some(t) = crate::work::room::scheduler::core::lookup_task_by_id(task.get()) {
                let same = Arc::ptr_eq(&t.ident.team, &ident.team);
                let mine = current()
                    .running_task()
                    .map(|me| me.heir(t.ident.team.id).is_some())
                    .unwrap_or(false);
                if !(same || mine) {
                    return ret_err(frame, GateError::Denied);
                }
            }
            // 挂起后恢复读到的 a0 = 挂起前预置值 ⇒ 预置 0（未回收）；当场判定再改写
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            match utask::join(task.get(), dur) {
                Ok(JoinStep::Dead) => frame.gpr.set_x(Gprs::A0, 1),
                Ok(JoinStep::Alive) => {}
                Ok(JoinStep::Switched(pa)) => return pa as *mut TrapContext,
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Control(ControlCall::Panic { code }) => {
            panic!("user-initiated panic (code {code:#x})");
        }
        EnvCall::Control(ControlCall::Backtrace { buf, frames }) => {
            // 用户自诊断回溯：采样当前任务用户栈（user_satp 根表，零锁不触缺页），
            // 把 pc 数组经 mail::copy_out 写进用户 buf。buf 非法（未映射/不可写）→
            // copy_out 返 false → A0 = 负值（EnvError）。
            let world = ident.team.space.kind();
            let sp = frame.gpr.x(Gprs::SP);
            let fp = frame.gpr.x(Gprs::S0);
            let mut reader = StackReader::new(frame.user_satp.ppn());
            let cfg = ResolveCfg::user(world, sp.saturating_add(frame::SPAN));
            // 域筛：候选 pc 是否属本域代码。符号表已移除：不再做符号命中域筛。
            let code = move |_w: usize| true;
            let (pc_arr, count) = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
            // 打包 pc 数组字节（仅前 min(count, frames) 帧），copy_out 写用户 buf。
            let keep = count.min(frames);
            let mut bytes = [0u8; frame::DEPTH * core::mem::size_of::<usize>()];
            for i in 0..keep {
                bytes[i * core::mem::size_of::<usize>()..][..core::mem::size_of::<usize>()]
                    .copy_from_slice(&pc_arr[i].pc.as_usize().to_le_bytes());
            }
            let ok = mail::copy_out(
                &ident.team.space,
                &bytes[..keep * core::mem::size_of::<usize>()],
                buf,
            );
            frame
                .gpr
                .set_x(Gprs::A0, if ok { keep } else { -1isize as usize });
        }
        EnvCall::Memory(MemoryCall::Mmap { size, at }) => {
            let size = size.max(1).next_multiple_of(PAGE_SIZE);
            let fixed = at.get();
            let va = {
                let s = &ident.team.space;
                if fixed == 0 {
                    ShareWindow::mmap(s, size).map(|span| span.va)
                } else {
                    let flags = s.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
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
        EnvCall::Mail(MailCall::UnsealHole { mtu }) => {
            // 编排创建：mail::hole::meta(mtu) 建实体 → gate::new_pie 建门闩 → 落 pies。
            // 门闩持资源实体的强引用——寿命即能力寿命。
            let r = (|| -> Result<usize, GateError> {
                let task = current().running_task().ok_or(GateError::Denied)?;
                let meta = mail::hole::meta(mtu, task.ident.id)?;
                let pie: Pie<mail::hole::HoleMeta> = gate::new_pie(
                    meta,
                    Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
                    None, // 原始自持：无 sire
                );
                let token = pie.token;
                task.pies.lock().push(AnyPie::Hole(pie));
                Ok(token)
            })();
            match r {
                Ok(token) => frame.gpr.set_x(Gprs::A0, token),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        EnvCall::Mail(MailCall::UnsealPole { bytes }) => {
            // 编排创建：mail::pole::meta() 建实体 → gate::new_pie 建门闩 → 落 pies →
            // auto-map 创建者视图（创建者 pie 全权 → R|W）。
            let r = (|| -> Result<usize, GateError> {
                let task = current().running_task().ok_or(GateError::Denied)?;
                let meta = mail::pole::meta(bytes, task.ident.id)?;
                let task_space = task.ident.team.space.clone();
                let pie: Pie<mail::pole::PoleMeta> = gate::new_pie(
                    meta.clone(),
                    Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
                    None, // 原始自持：无 sire
                );
                let token = pie.token;
                // 创建者自留 pie 全权 → map 走 R|W（U 位由空间策略决定）。
                let creator_flags = task_space.pte_policy(
                    PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::A | PteFlags::D,
                );
                mail::pole::map(&meta, token, &task_space, creator_flags)?;
                task.pies.lock().push(AnyPie::Pole(pie));
                Ok(token)
            })();
            match r {
                Ok(token) => frame.gpr.set_x(Gprs::A0, token),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        EnvCall::Mail(MailCall::Push { token, msg, len }) => {
            let token = token.get();
            let va = msg.get();
            let len = len;
            let task = current().running_task();
            let me = task.as_ref().map(|t| t.ident.id).unwrap_or(0);
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
                    AnyPie::Hole(p) => Some(Ok(p.meta().clone())),
                    _ => None,
                }
            }) {
                Some(Ok(meta)) => {
                    // 长度校验：必须 ≥1 且 ≤ hole.mtu（meta() 入口已校验 mtu∈[1,4096]）。
                    if len == 0 || len > meta.mtu {
                        Err(GateError::Denied)
                    } else {
                        // 锁外 copy_in 到堆暂存：slot = L3，Space.segments = L2，
                        // 持 L3 调 L2 是 4→2 反向嵌套，禁止。
                        let mut staging = alloc::vec![0u8; len];
                        if !mail::copy_in(&ident.team.space, &mut staging, va) {
                            Err(GateError::Denied)
                        } else {
                            // `me` = 推者身份：内核盖章，与消息同槽交付收方。
                            mail::hole::try_push(&meta, &staging, me)
                        }
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
        EnvCall::Mail(MailCall::Pull { token, buf, max }) => {
            let token = token.get();
            let va = buf.get();
            let max = max;
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
                    AnyPie::Hole(p) => Some(Ok(p.meta().clone())),
                    _ => None,
                }
            }) {
                Some(Ok(meta)) => {
                    if max == 0 || max > meta.mtu {
                        Err(GateError::Denied)
                    } else {
                        let mut staging = alloc::vec![0u8; max];
                        match mail::hole::try_pull(&meta, &mut staging) {
                            Ok((n, from)) => {
                                if !mail::copy_out(&ident.team.space, &staging[..n], va) {
                                    Err(GateError::Denied)
                                } else {
                                    Ok((n, from))
                                }
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
                Some(Err(e)) => Err(e),
                None => Err(GateError::Denied),
            };
            // 正路径：a0 = 实际长度、a1 = 发送者 task id；错误路径 a0 = 负码。
            match r {
                Ok((n, from)) => {
                    frame.gpr.set_x(Gprs::A0, n);
                    frame.gpr.set_x(Gprs::A1, from);
                }
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
        EnvCall::Mail(MailCall::Map { token }) => {
            let token = token.get();
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
                        Some(Ok((p.meta().clone(), token, flags)))
                    }
                    _ => None,
                }
            }) {
                Some(Ok((meta, token, Ok(flags)))) => mail::pole::map(
                    &meta,
                    token,
                    &ident.team.space,
                    ident.team.space.pte_policy(flags),
                ),
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
            let token = token.get();
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
                    AnyPie::Pole(p) => Some(Ok((p.meta().clone(), token))),
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
            // 封印：**只有资源开辟者**可做（`owner` 是 Meta 上的字段，O(1) 判定）。
            // 只置死 + 唤醒等待者——内存由引用归零回收（寿命 = 能力寿命）。
            let token = token.get();
            let me = match current().running_task() {
                Some(t) => t,
                None => return ret_err(frame, GateError::Denied),
            };
            let pie = {
                let pies = me.pies.lock();
                pies.iter().find(|p| p.token() == token).cloned()
            };
            let r = match pie {
                None => Err(GateError::Denied),
                Some(p) if !p.alive() => Err(GateError::Dead),
                Some(p) if p.owner() != Some(me.ident.id) => Err(GateError::Denied),
                Some(p) => {
                    match &p {
                        AnyPie::Hole(h) => mail::hole::seal(h.meta()),
                        AnyPie::Pole(pl) => mail::pole::seal(pl.meta()),
                    }
                    Ok(())
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
        EnvCall::Mail(MailCall::Accord { src, dst, subset }) => {
            let src_token = src.get();
            let dst_id = dst.get();

            let src_task = current().running_task();
            let src = match src_task.and_then(|t| {
                t.pies
                    .lock()
                    .iter()
                    .find(|p| p.token() == src_token)
                    .cloned()
            }) {
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
            // BACK 守门：带 BACK 只能授回 sire 的持有者（原始自持 None 不受限）。
            if !gate::vestable(&src, dst_id, &gate::snap()) {
                return ret_err(frame, GateError::Denied);
            }
            let target = match crate::work::room::scheduler::core::lookup_task_by_id_weak(dst_id) {
                Some(w) => w,
                None => return ret_err(frame, GateError::Denied),
            };
            let r = gate::accord(&src, &target, subset);
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(token) => token,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Narrow { token, subset }) => {
            let token = token.get();
            let task = current().running_task();
            let meta = match task.as_ref().and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.covers(subset) {
                    return Some(Err(GateError::Denied));
                }
                match pie {
                    AnyPie::Hole(p) => {
                        if !p.meta().alive() {
                            return Some(Err(GateError::Dead));
                        }
                        Some(Ok(None))
                    }
                    AnyPie::Pole(p) => Some(Ok(Some(p.meta().clone()))),
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
            let token = token.get();
            let caller = match current().running_task() {
                Some(t) => t,
                None => return ret_err(frame, GateError::Denied),
            };
            let target = match crate::work::room::scheduler::core::lookup_task_by_id_weak(dst_id) {
                Some(w) => w,
                None => return ret_err(frame, GateError::Denied),
            };
            let r = gate::revoke(&caller, &target, token, &gate::snap());
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(_) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Collect { index }) => {
            // 自省：报出本任务权限表第 index 份（token + permission + vestor）。
            // 越界 → token = 0、permission = 空、vestor = 0——哨兵不报错。
            //
            // a0 = token（usize），a1 = permission bits（低 32 位）| vestor task id（高 32 位）。
            // vestor = None 时内核编码为 `TaskId(0)`——哨兵与原「无 vestor」语义一致，
            // 因为 TaskId(0) 本来就是「无上下文」哨兵。
            //
            // 锁序：先克隆出 pie（放 pies 锁），再取快照求 vestor——两者都是 L3，
            // 绝不嵌套。
            let pie = current().running_task().and_then(|t| {
                let pies = t.pies.lock();
                pies.get(index).cloned()
            });
            let (token, perm_bits, vestor_id) = match pie {
                Some(p) => {
                    let v = gate::vestor(&p, &gate::snap()).unwrap_or(0);
                    (p.token(), p.permission().bits() as usize, v)
                }
                None => (0, 0, 0),
            };
            frame.gpr.set_x(Gprs::A0, token);
            frame
                .gpr
                .set_x(Gprs::A1, (vestor_id << 32) | (perm_bits & 0xffff_ffff));
        }
        EnvCall::Mail(MailCall::Owned { token }) => {
            // 查询：本任务表里这枚门闩的「授与人 + 资源开辟者」。
            // 与 Collect 分工：Collect 按索引枚举（发现未见过的句柄），
            // Owned 按句柄查事实（vestor = 父门闩的持有者，owner 随资源不变）。
            //
            // a0 = vestor（None → 0），a1 = owner。资源已封印 → Dead（开辟者答不出）。
            // 锁序：先克隆出 pie（放 pies 锁），再取快照求 vestor。
            let token = token.get();
            let pie = current().running_task().and_then(|t| {
                let pies = t.pies.lock();
                pies.iter().find(|p| p.token() == token).cloned()
            });
            let r = match pie {
                Some(p) => match p.owner() {
                    Some(owner) => Ok((gate::vestor(&p, &gate::snap()).unwrap_or(0), owner)),
                    None => Err(GateError::Dead),
                },
                None => Err(GateError::Denied),
            };
            match r {
                Ok((vestor_id, owner_id)) => {
                    frame.gpr.set_x(Gprs::A0, vestor_id);
                    frame.gpr.set_x(Gprs::A1, owner_id);
                }
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Mail(MailCall::Release { token }) => {
            // 自释：放下自己的一份门闩（含全部后代；无权限要求；Pole 同步 unmap）。
            let token = token.get();
            let r = match current().running_task() {
                Some(task) => gate::release(&task, token, &gate::snap()),
                None => Err(GateError::Denied),
            };
            frame.gpr.set_x(
                Gprs::A0,
                match r {
                    Ok(_) => 0,
                    Err(e) => e.code() as usize,
                },
            );
        }
        EnvCall::Mail(MailCall::Wait { token, dir, millis }) => {
            // 锁内解析 token → Arc<HoleMeta>：pies 与 wait_sites 同为 L3，绝不嵌套；
            // `running_task` 的临时强引用在闭包内即 drop，不跨挂起。
            let token = token.get();
            let need = match dir {
                HoleDir::Pull => Need::Read,
                HoleDir::Push => Need::Write,
            };
            let resolved = match current().running_task().and_then(|t| {
                let pies = t.pies.lock();
                let pie = pies.iter().find(|p| p.token() == token)?;
                if !pie.allows(need) {
                    return Some(Err(GateError::Denied));
                }
                if !pie.alive() {
                    return Some(Err(GateError::Dead));
                }
                match pie {
                    AnyPie::Hole(p) => Some(Ok(p.meta().clone())),
                    _ => None,
                }
            }) {
                Some(r) => r,
                None => Err(GateError::Denied),
            };
            let dur = if millis == usize::MAX {
                Duration::MAX
            } else {
                Duration::from_millis(millis as u64)
            };
            match resolved {
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
                Ok(meta) => {
                    // 挂起路径的默认返回 = false（未当场就绪）；可能 halt 的分支先放身份。
                    frame.gpr.set_x(Gprs::A0, 0);
                    drop(ident);
                    match mail::hole::wait(&meta, dir, dur) {
                        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
                        Ok(mail::hole::Waited::Resume(true)) => frame.gpr.set_x(Gprs::A0, 1),
                        Ok(mail::hole::Waited::Resume(false)) => {}
                        Ok(mail::hole::Waited::Parked(Some(pa))) => {
                            return pa as *mut TrapContext;
                        }
                        Ok(mail::hole::Waited::Parked(None)) => return run() as *mut TrapContext,
                    }
                }
            }
        }
    };
    frame as *mut TrapContext
}
