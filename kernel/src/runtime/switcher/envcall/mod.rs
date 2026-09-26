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
// 返回值写回 a0（`Gprs::A0`）；sepc 按**实际指令长度**前进（RVC 2 / 标准 4 字节，
// 见 `instr_len`；Reap 不返回）。
// 时间语义统一以毫秒（Duration 边界）表达（Park / Wait）；Ticks 仅作兼容诊断。
// 调用名与调度词族同词：Starve/Park/Wait/Wake 分别直呼
// `scheduler::core::starve` / `messenger::{park, wait, wake}`；同一个 `messenger` 面上的
// `join`/`fall` 也在本层。**Reap 是反向的**：本层只置退出原因并返 `None`（空指针），
// `quit` 由退场窄尾那一帧调（`trap` 的 `Reap` 分支；服务面转发层已随内核任务面一并删除）。

use core::time::Duration;

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{ChronoCall, DebugCall, EnvCall, RoomCall, RoomFail, TaskId, UnitCall, UnitFail};

use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::memory::manager::entry::PteFlags;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::messenger::{self, Handoff, WakeKey, park, park_until, wait, wake};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::gate::Permission;
use crate::work::unit::life::TaskLife;
use crate::work::unit::source::Source;
use crate::work::unit::space::{Space, SpaceKind};
use crate::work::unit::task::{MAX_ARGS, Task, TaskIdent, TaskTag};
use crate::work::unit::team::UnitError;
use crate::work::unit::weak::{Site, TaskWeak};

mod control;
mod debug;
mod mail;
mod memory;
mod pie;
mod tole;

/// 报文对账开关：`DebugCall::SetTrace` 写（每域各自一份静态——域是独立地址空间，
/// 开关不跨域）。**今天没有读者**：原先读它的是 `Port::call`，那一层已随 Port 那轮
/// 搬去各协议；用户侧那份 `env::debug::tracing()` 因此闲置。
/// 只在调试时打开——布局错位这类病只有真实字节能证。
pub static TRACE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Permission 子集 → PteFlags（cap ⊆ 页表的翻译：subset 决定页表实际权限）。
///
/// | subset                  | PteFlags                |
/// |-------------------------|-------------------------|
/// | FETCH                    | V\|R\|A\|D              |
/// | FETCH \| STORE           | V\|R\|W\|A\|D           |
/// | other（含空 / 仅 STORE）| Denied                  |
///
/// U 位不在此处决定——由目标空间的 [`Space::pte_policy`] 加。
fn subset_to_pte(subset: Permission) -> Result<PteFlags, env::PieFail> {
    if !subset.contains(Permission::FETCH) {
        return Err(env::PieFail::Denied);
    }
    let mut f = PteFlags::V | PteFlags::A | PteFlags::D;
    f |= PteFlags::R;
    if subset.contains(Permission::STORE) {
        f |= PteFlags::W;
    }
    Ok(f)
}

/// 写回错误码并返回待恢复帧。
///
/// **泛型**：收的是**域词表**（`env::FailCode` 的共同部分只有"码"）。九域各自一枚枚举，
/// 这里不做任何折算——折算在**产生错误的那一处**（如 `memory.rs` 的 `From<MapError>`、
/// `gate/` 的 `GateFail` 构造子）。
fn ret_err<E: env::FailCode>(frame: &mut TrapContext, e: E) -> *mut TrapContext {
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
fn map_err(e: crate::memory::manager::MapError) -> UnitFail {
    match e {
        crate::memory::manager::MapError::OutOfMemory => UnitFail::OoM,
        // 其余（对齐 / 已映射 / 未映射 / 无区段 / 借入加宽 / 段状态不符 / 恒等压栈）在
        // "产线程的栈与帧"这条路上都是"要的东西给不出" ⇒ `Denied`。**穷尽 match**：
        // `MapError` 多一枚变体就编不过（同一个内核错误在 Memory 域另有一处折算）。
        crate::memory::manager::MapError::NotAligned
        | crate::memory::manager::MapError::AlreadyMapped
        | crate::memory::manager::MapError::NotMapped
        | crate::memory::manager::MapError::NoRegion
        | crate::memory::manager::MapError::WidenDenied
        | crate::memory::manager::MapError::SegmentMismatch
        | crate::memory::manager::MapError::DramOverlap => UnitFail::Denied,
    }
}

/// 读调用方空间里的 `count` 个字（`Spawn` 的启动参数）。
///
/// 缓冲**定长在栈上**（`MAX_ARGS · 8` = 512 B）：`count` 的界就是 `MAX_ARGS`，不必为
/// 它分配。源与 `Build` 同一件——见 [`Source`]。
fn copy_words(space: &Space, va: KVirt, count: usize) -> Option<Vec<usize>> {
    if count > MAX_ARGS {
        return None;
    }
    let width = size_of::<usize>();
    let len = count * width;
    let mut bytes = [0u8; MAX_ARGS * size_of::<usize>()];
    let src = Source::Space { space, va, len };
    if !src.read(0, &mut bytes[..len]) {
        return None;
    }
    // 同 `Source::read` 那条：**可失败，不 panic**——`Vec::with_capacity` 走
    // `handle_alloc_error`（内存吃紧 ⇒ 整机 halt），失败与"区间未映射"同路返回
    // `None`，调用方把两类都落到 `Denied` 上，机器照旧活着。
    let mut out: Vec<usize> = Vec::new();
    out.try_reserve(count).ok()?;
    for i in 0..count {
        let mut w = [0u8; size_of::<usize>()];
        w.copy_from_slice(&bytes[i * width..(i + 1) * width]);
        out.push(usize::from_le_bytes(w));
    }
    Some(out)
}

/// envcall 分发。
///
/// 入参 frame = 当前任务用户帧；`ident` = 当前任务身份（**Arc 所有权移交**——
/// 可能触发 halt 的分支（Reap/Park/Wait → run）须先 `drop(ident)`，否则 halt
/// 时身份 Arc 仍持最后任务 team → space 不 drop，关机审计误报帧泄漏）。
/// 返回 `Some(帧)` = 待恢复帧（Starve/Park 给下一任务帧；其余给本次 frame）；
/// `None` = **本任务退场**——由调用方（`trap_handler`）在最浅的 Rust 帧里收尾。
///
/// 「退场不由本函数做」是**退场窄尾**：任务退场＝上下文被切走、
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
        Err(_) => return ret_err(frame, env::DispatchFail::Unknown),
    };
    match envcall {
        EnvCall::Room(RoomCall::Starve) => return current().starve() as *mut TrapContext,
        EnvCall::Room(RoomCall::Reap { reason, note, len }) => {
            // 本任务退场：**不在这里 quit**（见 [`dispatch`] 的退场窄尾）——空指针即标记。
            //
            // `reason` 是**数据**：0 = 自愿/正常结束，非 0 = 域自己的诊断编号。内核只把
            // 它记进 trace，**不解释**——"域为什么不可续"是域的判断，内核的事只是
            // "它不再续跑"与"把它的账结清"。故本仓**没有** `ControlCall::Panic`
            // 这样的第二入口：那会把域的策略写进 ABI，并让"任务终止"
            // 这条不变量在 ABI 里有两个出口。
            //
            // 写进逐核暂存槽，由 `quit` 统一发出 `RoomEvent::Exit`：那是**所有**退出
            // 路径（Reap / 故障隔离 / doom 级联）的公共点，事件因此只发一次、
            // 且每条路径都带得上原因（故障路径带走的是内核给的原因码）。
            //
            // `note` = 域自己带的一句话（`len = 0` = 无话）：这里只落下它的**位置**——
            // 打印与入账都在 `quit`（退场路径的公共点），而读它的时点仍在
            // `reap`/`bury` 之前，那段空间还在（见 `messenger::EXIT_NOTE`）。
            crate::work::room::messenger::set_exit_note(note.get(), len);
            crate::work::room::messenger::set_exit_reason(reason);
            drop(ident);
            return core::ptr::null_mut();
        }
        EnvCall::Room(RoomCall::Doom { task }) => {
            // 他杀（与 `Reap` 成对：自杀 ↔ 他杀）。判据只有**判活**，**没有血缘门**
            // ——这是 `doom` 口径：收一个域是"命令"，不是"血缘特权"。
            //
            // 曾经这里要 `descends`（目标域得在我后代链里）。删掉它的理由是判据分家：
            // 内核只回答"能不能收"（能），"该不该收"归 `protocol::system` 的编排者
            // （它拿服务表说话）。与建域那一支是同一次分家，见 `UnitCall::Build`。
            //
            // 代价照实记：**服务之间因此没有护栏**（任何域都能拆任何域）。收窄只能在
            // 编排侧做（"谁能申请收谁"），不是内核该长的东西。
            //
            // 语义仍是**域粒度**：`task` 只是"指认域"的手柄，它所属的域连同子树一起走
            // （同域的线程一并，不会剩半个域）——执行复用结构面既有的两相扑杀。
            //
            // 一次调用**只下一道令**，不下场等它回收：要等就 `UnitCall::Join`
            // （Linux 的 `kill` 也是"送到即回"）。
            let target = muster(task).and_then(|w| w.upgrade());
            let Some(target) = target else {
                // 名册升不起来 = 从未入册 / 已回收——与 `Join` 判活三态同一口径。
                return ret_err(frame, RoomFail::Dead);
            };
            let team = target.ident.team.clone();
            // **判活是域粒度**：域里已没有还没收尾的线程 ⇒ 与"名册升不起"同答 `Dead`，
            // 不再"答成功却什么都没做"（读法与 `Team::all_reaped` 同一句）。
            // 空域够不到这一支——它没有 `TaskId` 手柄，那条边界照旧（见 `protocol::system` §八）。
            if team.all_reaped() {
                return ret_err(frame, RoomFail::Dead);
            }
            // 下令时记一笔（谁杀的）；死亡时受害者那颗核另记 `Exit { EXIT_DOOM }`
            // ——两条分开是因为它们落在不同的核上（见 `RoomEvent::Doomed`）。
            trace::note(EventKind::Room(RoomEvent::Doomed {
                tid: target.ident.id.get(),
                by: ident.id.get(),
            }));
            drop(target);
            drop(ident);
            messenger::cull(&[team], messenger::EXIT_DOOM);
            frame.gpr.set_x(Gprs::A0, 0);
        }
        EnvCall::Chrono(ChronoCall::Ticks) => {
            frame.gpr.set_x(Gprs::A0, timer::ticks() as usize);
        }
        EnvCall::Debug(DebugCall::Put { buf, len }) => {
            return debug::put(frame, &ident, buf.get(), len);
        }
        EnvCall::Debug(DebugCall::Get { buf, len }) => {
            return debug::get(frame, &ident, buf.get(), len);
        }
        EnvCall::Debug(DebugCall::SetTrace { on }) => {
            frame.gpr.set_x(Gprs::A0, debug::set_trace(on));
        }
        EnvCall::Room(RoomCall::Park { millis }) => {
            drop(ident);
            match park::<RoomFail>(Duration::from_millis(millis as u64)) {
                Ok(pa) => return pa as *mut TrapContext,
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Room(RoomCall::ParkUntil { at }) => {
            drop(ident);
            match park_until::<RoomFail>(at) {
                // 到点已过 ⇒ 未离核即续跑（ABI 契约：当场返回，不是让出一拍）。
                Ok(None) => {}
                Ok(Some(pa)) => return pa as *mut TrapContext,
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Room(RoomCall::Wait { key, millis }) => {
            // 键 → 存活单元：**解析在调用方这一层**（room 不认识注册表）。空间键的
            // 寿命就是本任务所属空间的寿命，故弱引用随键一起交给等待机。
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.asid(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let dur = millis.into_duration();
            drop(ident);
            // `wlife` **按值**交给等待机（站点是它唯一的持有者）。
            match wait::<RoomFail>(wkey, wlife, dur) {
                // `RoomCall::Wait` 没有当场结论：未离核即续跑。
                Ok(Handoff::Resume(())) => {}
                Ok(Handoff::Switch(pa)) => return pa as *mut TrapContext,
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Room(RoomCall::Wake { key }) => {
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.asid(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let woke = wake(wkey, &wlife);
            frame.gpr.set_x(Gprs::A0, woke as usize);
        }
        EnvCall::Chrono(ChronoCall::Clock) => {
            // 单字纳秒（自启动基准）：`u128 → u64` 饱和（584 年，实际到不了）。
            let ns = clock::uptime().as_nanos().min(u64::MAX as u128) as u64;
            frame.gpr.set_x(Gprs::A0, ns as usize);
        }
        // Memory 域的臂整个在 `memory.rs`：五格 + **一处** `MapError → MemoryFail` 折算。
        EnvCall::Memory(call) => {
            memory::dispatch(frame, call, &ident);
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
                    None => return ret_err(frame, UnitFail::Denied),
                }
            };
            // 启动参数：从调用方空间拷（count == 0 → 空）
            let words = match copy_words(&ident.team.space, KVirt::from_raw(args.get()), count) {
                Some(w) => w,
                None => return ret_err(frame, UnitFail::Denied),
            };
            // entry = 0 → 域默认入口（`Build` 装载所得 e_entry）
            let entry_va = if entry == 0 {
                target.default_entry()
            } else {
                entry
            };
            let mut builder = target.task().entry(KVirt::from_raw(entry_va)).args(words);
            if stack > 0 {
                builder = builder.stack(stack);
            }
            // 恒产 Held：授权顺序由父方 `Accord` → `Hatch` 保证
            match builder.hold() {
                Ok(t) => frame.gpr.set_x(Gprs::A0, t.ident.id.get()),
                Err(e) => return ret_err(frame, map_err(e)),
            }
        }
        EnvCall::Unit(UnitCall::SelfId) => {
            let id = current()
                .running_task()
                .map(|t| t.ident.id)
                .unwrap_or(TaskId::new(0));
            frame.gpr.set_x(Gprs::A0, id.get());
        }
        EnvCall::Unit(UnitCall::Sire) => {
            // 溯源：生我者的 task id。0 = 顶级域（boot）或父已亡。
            let id = current()
                .running_task()
                .and_then(|t| t.ident.team.sire())
                .unwrap_or(TaskId::new(0));
            frame.gpr.set_x(Gprs::A0, id.get());
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
        EnvCall::Unit(UnitCall::Build { elf, len, kind }) => {
            // 门：**没有门**（Mint 口径）。
            //
            // 曾经这里是"建域权就是 S 态"——那是**内核替调用方定"该不该起"的时候**
            // 留下的。判据现在已经分家：能不能起由本层回答（答"能"），该不该起归
            // `protocol::system` 的编排者（它拿服务表与策略说话）。
            //
            // 放开它**不构成提权**：提权的两条路都还堵着——特权级由内核打包表决定
            // （调用方说不上话），镜像仍要调用方交字节（Mint-lite 口径，见该协议正文）。
            //
            // 镜像：**不拷**。`Source` 把"这份字节从哪来"交给 loader 逐段现取——一份
            // ELF 里 97% 是符号表与调试信息（实测 18 台 debug 镜像：78.2 MiB 文件、
            // 2.13 MiB 段实体），从前先整份搬进内核暂存再丢掉，是白搬的那一跳。
            let source = Source::Space {
                space: &ident.team.space,
                va: KVirt::from_raw(elf.get()),
                len,
            };
            // sire = 调用方：`build` 内部闭合血缘（域必入我 heir）。
            //
            // 这枚弱引用的出身是**血亲**（`Site::Sire`）：它几步之后就会住进
            // `Team.sire`，中间没有挂起点（`build` 全程不 switch）。写清出身是为了
            // 弱引用收支账能把这枚与"抄件"分开（见 `work::unit::weak`）。
            let sire = match current().running_task() {
                Some(me) => TaskWeak::stored(Arc::downgrade(&me), Site::Sire),
                None => TaskWeak::empty(),
            };
            match crate::work::unit::build(&source, SpaceKind::from(kind), sire) {
                Ok(team) => frame.gpr.set_x(Gprs::A0, team.id.get()),
                // 源读不到 = 调用方自己的映射不在（或本域另一枚线程刚放手）——与从前
                // "暂存拷不进来"同一个负码。
                Err(UnitError::Unreadable) => return ret_err(frame, UnitFail::Denied),
                // 内存不够从"镜像不认"里分出来：`OoM` 这一格编排者本来就接
                // （`protocol::system::core::Fail::Full`），`BadImage` 没有。
                Err(UnitError::OoM) => return ret_err(frame, UnitFail::OoM),
                Err(UnitError::Load) => return ret_err(frame, UnitFail::BadImage),
            }
        }
        EnvCall::Unit(UnitCall::Hatch { task }) => {
            let target = match muster(task).and_then(|w| w.upgrade()) {
                Some(t) => t,
                None => return ret_err(frame, UnitFail::Denied),
            };
            // 授权：与我同域，或属于我 heir 里的子域
            let same = Arc::ptr_eq(&target.ident.team, &ident.team);
            let mine = current()
                .running_task()
                .map(|me| me.heir(target.ident.team.id).is_some())
                .unwrap_or(false);
            if !(same || mine) {
                return ret_err(frame, UnitFail::Denied);
            }
            if let Err(e) = Task::release(&target) {
                return ret_err(frame, e);
            }
        }
        EnvCall::Unit(UnitCall::Join { task, millis }) => {
            let dur = millis.into_duration();
            // 判活三态**在边界一次问清**（room 不查注册表）：
            //   ① 名册里没有这个 id ⇒ **从未分配** = 非法 id ⇒ Denied。旧版把这一支与
            //      「目标仍活」折在一起（判活有两个真相源时必然如此），于是非法 id 拿到
            //      「未回收」、`Join{0}` 拿到「已回收」；而那个本该拦它的 `Err(Denied)`
            //      需要 `target_dead ∧ ¬allocated` 同时成立，两条来路都蕴含 `allocated`
            //      ⇒ 它**曾经永远不可达**。
            //   ② 升不起强引用 ⇒ 已消失（对象已回收）⇒ 当场结论「已回收」；授权无从核对
            //      （照旧放行；寿命无从谈起 ⇒ 空弱引用，站点当场判死、不建站点）。
            //   ③ 仍是活任务 ⇒ 当场核对授权，并把「退出钩子是否已跑完」读出来。
            let Some(target) = muster(task) else {
                return ret_err(frame, UnitFail::Denied);
            };
            // **挂起前放掉那枚抄件**（`muster` 抄出来的弱引用）：`target` 只用来当场判活
            // 与取 `(reaped, life)`，此后它就是一具"跨挂起还压在栈上"的引用 —— 而
            // `messenger::join` 会挂起本任务。这条链一旦被别核判死（或被收尾就地冻住），
            // `target` 的 `Drop` 永不执行：目标任务的 `ArcInner` 外壳（152 B）被一枚
            // **永远活着**的弱引用扣住 ⇒ 关机审计 `leak: task 1`（`strong 0 weak 1`）。
            // 与上面那行 `drop(ident)` 是同一条纪律（见 `dispatch` 头注）；弱引用同样
            // 算"引用"，只是它钉住的是外壳而不是载荷。
            let (reaped, life) = match target.upgrade() {
                Some(t) => {
                    let same = Arc::ptr_eq(&t.ident.team, &ident.team);
                    let mine = current()
                        .running_task()
                        .map(|me| me.heir(t.ident.team.id).is_some())
                        .unwrap_or(false);
                    if !(same || mine) {
                        return ret_err(frame, UnitFail::Denied);
                    }
                    (t.tag() == TaskTag::Reaped, t.life())
                }
                None => (true, Weak::new()),
            };
            // 挂起后恢复读到的 a0 = 挂起前预置值 ⇒ 预置 0（未回收）；当场判定再改写
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            drop(target);
            match messenger::join::<UnitFail>(TaskLife { id: task, life }, reaped, dur) {
                // 未离核：当场结论（true = 调用开始时目标已回收）。
                Ok(Handoff::Resume(dead)) => frame.gpr.set_x(Gprs::A0, dead as usize),
                Ok(Handoff::Switch(pa)) => return pa as *mut TrapContext,
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Unit(UnitCall::Fall { millis }) => {
            let dur = millis.into_duration();
            // **只等"我自己这张表"**：键由内核从调用者推出来，故这里没有参数、
            // 也就没有伪造面（同 `SelfId` / `Sire` 那一路）。
            let Some(me) = current().running_task() else {
                return ret_err(frame, UnitFail::Busy);
            };
            let mine = TaskLife {
                id: me.ident.id,
                life: me.life(),
            };
            // 挂起后恢复读到的 a0 = 挂起前预置值 ⇒ 预置 0（没落过）；当场判定再改写。
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            // 跨挂起不得持强引用（同 `Join` 那条纪律）：只留那份弱引用。
            drop(me);
            match messenger::fall::<UnitFail>(mine, dur) {
                Ok(Handoff::Resume(landed)) => frame.gpr.set_x(Gprs::A0, landed as usize),
                Ok(Handoff::Switch(pa)) => return pa as *mut TrapContext,
                Err(e) => return ret_err(frame, e),
            }
        }
        EnvCall::Unit(UnitCall::Oust { team }) => {
            let Some(me) = current().running_task() else {
                return ret_err(frame, UnitFail::Denied);
            };
            // 凭证就是**我自己那张血缘表**（同 `Spawn` 的门）：查到 = 我是它的 sire。
            let Some(child) = me.heir(team) else {
                return ret_err(frame, UnitFail::Denied);
            };
            // 前置：域里没有还没收尾的线程（判据读法与"回收对调用方不可观测"那条一致）。
            if !child.all_reaped() {
                return ret_err(frame, UnitFail::Busy);
            }
            // 手里那份瞬时引用先还掉：摘除只需 id，析构留给锁外。
            drop(child);
            // **摘除与析构分开**：`oust` 在表锁内只做 Vec 摘除，交回的那一份在这里落地
            // ⇒ `Team`（连带 `Space`）的析构不在 L3 锁里走。
            drop(me.oust(team));
        }
        // Control 域的臂整个在 `control.rs`（回溯采样 + 一处 `ControlFail`）。
        EnvCall::Control(call) => {
            control::dispatch(frame, call, &ident);
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
        // 多路等待轴：四动词里只有 `Await` 可能换帧，故与数据轴一样带 `Park`。
        EnvCall::Tole(call) => {
            if let Some(out) = tole::dispatch(frame, call, ident) {
                return match out {
                    mail::Outcome::Resume => frame as *mut TrapContext,
                    mail::Outcome::Park(next) => next,
                };
            }
        }
    };
    frame as *mut TrapContext
}
