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
// 调用名与调度词族同词：Starve/Park/Reap/Wait/Wake 分别直呼
// `scheduler::core::starve` / `messenger::{park, quit, wait, wake}`（服务面转发层
// 已随内核任务面一并删除）。

use core::time::Duration;

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{ChronoCall, ControlCall, EnvCall, IOCall, MemoryCall, Name, RoomCall, UnitCall};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::frame::{self, ResolveCfg, StackReader};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::messenger::{self, Handoff, WakeKey, park, wait, wake};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::{self, GateError, Permission};
use crate::work::unit::life::TaskLife;
use crate::work::unit::space::window::{HeapWindow, ShareWindow};
use crate::work::unit::space::{Pending, PendingState, Space, SpaceKind};
use crate::work::unit::task::{MAX_ARGS, Task, TaskIdent, TaskTag};

mod mail;
mod pie;

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
/// 返回 `Some(帧)` = 待恢复帧（Starve/Park 给下一任务帧；其余给本次 frame）；
/// `None` = **本任务退场**——由调用方（`trap_handler`）在最浅的 Rust 帧里收尾。
///
/// 「退场不由本函数做」是**退场窄尾**（§10.12 的修法 2）：任务退场＝上下文被切走、
/// 栈上的活引用随栈一起释放而**永不递减计数**。本函数的帧里带着它全部的临时值，
/// 在这里 `quit()` 就等于把它们的引用计数一起丢掉；把退场判断交回去之后，本函数的
/// 帧**先正常归还**（局部量照常 drop），只有 `trap_handler` 那一帧（此刻手里只有
/// frame/几个标量，`ident` 已移交）随 `restore` 被丢掉。
pub fn dispatch(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> Option<*mut TrapContext> {
    // 非空指针 = 待恢复帧；空指针 = 本任务退场（见 [`dispatch_inner`] 的 Reap 分支）。
    let pa = dispatch_inner(frame, ident);
    if pa.is_null() { None } else { Some(pa) }
}

/// 分发本体（见 [`dispatch`] 的文档：退场不由本层做，故本层的帧会正常归还）。
fn dispatch_inner(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> *mut TrapContext {
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
    // 未知调用号 = 调用方的错误（`a7` 由 U 态完全控制：空号 idx、已删 class 都落这里），
    // 按被拒绝处理并续跑调用方——与其它用户引起的异常同走故障隔离，绝不 panic
    // （panic 即 U 态一发 ebreak 打死整机）。想主动终止有正规原语 `RoomCall::Reap`。
    let envcall = match EnvCall::from_wire(number, &regs) {
        Ok(c) => c,
        Err(_) => return ret_err(frame, GateError::Denied),
    };
    match envcall {
        EnvCall::Room(RoomCall::Starve) => return current().starve() as *mut TrapContext,
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
                    // 无输入 = 条件未就绪（非阻塞原语的可重试信号），不是资源死。
                    // 走统一错误表：fid.rs 的 ABI 注释与 ecall.rs 的 D1 表都写 -3，
                    // 且用户侧 `EnvError::is_busy()` 就是判 -3（原先返 -2 使该判据永假）。
                    None => GateError::Busy.code() as usize,
                },
            );
        }
        EnvCall::Room(RoomCall::Reap { reason }) => {
            // 本任务退场：**不在这里 quit**（见 [`dispatch`] 的退场窄尾）——空指针即标记。
            //
            // `reason` 是**数据**：0 = 自愿/正常结束，非 0 = 域自己的诊断编号。内核只把
            // 它记进 trace，**不解释**——"域为什么不可续"是域的判断，内核的事只是
            // "它不再续跑"与"把它的账结清"。故本仓**没有** `ControlCall::Panic`
            // 这样的第二入口（§10.36）：那会把域的策略写进 ABI，并让"任务终止"
            // 这条不变量在 ABI 里有两个出口。
            //
            // 写进逐核暂存槽，由 `quit` 统一发出 `RoomEvent::Exit`：那是**所有**退出
            // 路径（Reap / 故障隔离 / doom 级联）的公共点，事件因此只发一次、
            // 且每条路径都带得上原因（故障路径带走的是内核给的原因码）。
            crate::work::room::messenger::set_exit_reason(reason);
            drop(ident);
            return core::ptr::null_mut();
        }
        EnvCall::Chrono(ChronoCall::Ticks) => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        EnvCall::Room(RoomCall::Park { millis }) => {
            drop(ident);
            return park(Duration::from_millis(millis as u64)) as *mut TrapContext;
        }
        EnvCall::Room(RoomCall::Wait { key, millis }) => {
            // 键 → 存活单元：**解析在调用方这一层**（room 不认识注册表）。空间键的
            // 寿命就是本任务所属空间的寿命，故弱引用随键一起交给等待机。
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.asid().get(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let dur = if millis == usize::MAX {
                Duration::MAX
            } else {
                Duration::from_millis(millis as u64)
            };
            drop(ident);
            match wait(wkey, &wlife, dur) {
                // `RoomCall::Wait` 没有当场结论：未离核即续跑。
                Handoff::Resume(()) => {}
                Handoff::Switch(pa) => return pa as *mut TrapContext,
            }
        }
        EnvCall::Room(RoomCall::Wake { key }) => {
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.asid().get(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let woke = wake(wkey, &wlife);
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
                    // 种类 = UserHeap：键是 `(asid, 页索引)` 而非地址，随空间
                    // `retire` 作废——on_alloc 已把种类记进账本，无需再 tag。
                    crate::memory::allocator::fence::on_alloc(
                        key,
                        size,
                        crate::memory::allocator::fence::Kind::UserHeap,
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
                        crate::memory::allocator::fence::Kind::UserHeap,
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
            build,
        }) => {
            // 门一：**建域权**——调用方自己表里必须有一枚活着的 `Nole`（存在权的载体）。
            // 按 token 在**调用方表里**找，故 token 不自证、借来的 token 无效。
            let holds = current()
                .running_task()
                .is_some_and(|me| gate::holds_build_right(&me, build.get()));
            if !holds {
                return ret_err(frame, GateError::Denied);
            }
            // 门二：S 态兜底（理由见 `fid.rs` 该 variant 的注释：能力管"谁有权"，
            // S 态管"血缘树能不能伸进沙箱外"）。
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
            let target = match muster(task.get()).and_then(|w| w.upgrade()) {
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
            // 判活三态**在边界一次问清**（room 不查注册表）：
            //   ① 名册里没有这个 id ⇒ **从未分配** = 非法 id ⇒ Denied。旧版把这一支与
            //      「目标仍活」折在一起（判活有两个真相源时必然如此），于是非法 id 拿到
            //      「未回收」、`Join{0}` 拿到「已回收」；而那个本该拦它的 `Err(Denied)`
            //      需要 `target_dead ∧ ¬allocated` 同时成立，两条来路都蕴含 `allocated`
            //      ⇒ 它**曾经永远不可达**。
            //   ② 升不起强引用 ⇒ 已消失（对象已回收）⇒ 当场结论「已回收」；授权无从核对
            //      （照旧放行；寿命无从谈起 ⇒ 空弱引用，站点当场判死、不建站点）。
            //   ③ 仍是活任务 ⇒ 当场核对授权，并把「退出钩子是否已跑完」读出来。
            let Some(target) = muster(task.get()) else {
                return ret_err(frame, GateError::Denied);
            };
            let (reaped, life) = match target.upgrade() {
                Some(t) => {
                    let same = Arc::ptr_eq(&t.ident.team, &ident.team);
                    let mine = current()
                        .running_task()
                        .map(|me| me.heir(t.ident.team.id).is_some())
                        .unwrap_or(false);
                    if !(same || mine) {
                        return ret_err(frame, GateError::Denied);
                    }
                    (t.tag() == TaskTag::Reaped, t.life())
                }
                None => (true, Weak::new()),
            };
            // 挂起后恢复读到的 a0 = 挂起前预置值 ⇒ 预置 0（未回收）；当场判定再改写
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            match messenger::join(
                TaskLife {
                    id: task.get(),
                    life,
                },
                reaped,
                dur,
            ) {
                // 未离核：当场结论（true = 调用开始时目标已回收）。
                Handoff::Resume(dead) => frame.gpr.set_x(Gprs::A0, dead as usize),
                Handoff::Switch(pa) => return pa as *mut TrapContext,
            }
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
            let ok = crate::work::mail::copy_out(
                &ident.team.space,
                &bytes[..keep * core::mem::size_of::<usize>()],
                buf,
            );
            // 用户 buf 非法（未映射 / 不可写）→ 统一错误表（写裸 -1 与 `Denied` 同值，
            // 但把「通道」写死在一处：D1 负码的单一真相是 `GateError::code`）。
            frame.gpr.set_x(
                Gprs::A0,
                if ok {
                    keep
                } else {
                    GateError::Denied.code() as usize
                },
            );
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
        // 两条轴各自成模块；命中的臂直接解构，未命中回落到下一个 match 腿。
        EnvCall::Mail(call) => {
            if let Some(out) = mail::dispatch(frame, call, ident) {
                return match out {
                    mail::Outcome::Resume => frame as *mut TrapContext,
                    mail::Outcome::Park(next) => next,
                };
            }
        }
        EnvCall::Pie(call) => {
            if let Some(pie::Outcome::Resume) = pie::dispatch(frame, call, ident) {
                return frame as *mut TrapContext;
            }
        }
    };
    frame as *mut TrapContext
}
