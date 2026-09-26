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
// `join`/`fall` 在 Unit 域。**Reap 是反向的**：它只置退出原因并答退场（空指针），
// `quit` 由退场窄尾那一帧调（`trap` 的 `Reap` 分支；服务面转发层已随内核任务面一并删除）。

use alloc::sync::Arc;

use env::{DebugCall, EnvCall};

use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::Space;
use crate::work::unit::task::TaskIdent;

// **一域一模块**：域的本体、折算与开关一律住该域的 `{域}.rs`。本文件只有三件——
// 过线的 decode（`EnvCall::from_wire` / `instr_len`）、下面那张一域一格的分派表、
// 以及回帧（`ret_err`）。
mod chrono;
mod control;
mod debug;
mod mail;
mod memory;
mod pie;
mod room;
mod tole;
mod unit;

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
    // 非空指针 = 待恢复帧；空指针 = 本任务退场。
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
        // 一域一格。域的本体各住 `{域}.rs`；本表只把落点翻译成"归还哪一帧"。
        EnvCall::Room(call) => match room::dispatch(frame, call, ident) {
            room::Outcome::Resume => {}
            room::Outcome::Switch(pa) => return pa,
            room::Outcome::Exit => return core::ptr::null_mut(),
        },
        EnvCall::Unit(call) => match unit::dispatch(frame, call, ident) {
            unit::Outcome::Resume => {}
            unit::Outcome::Switch(pa) => return pa,
        },
        // Memory 域的臂整个在 `memory.rs`：五格 + **一处** `MapError → MemoryFail` 折算。
        EnvCall::Memory(call) => {
            memory::dispatch(frame, call, &ident);
        }
        EnvCall::Chrono(call) => chrono::dispatch(frame, call),
        // 两条轴各自成模块；命中的臂直接解构，未命中回落到下一个 match 腿。
        EnvCall::Mail(call) => {
            if let Some(out) = mail::dispatch(frame, call, ident) {
                return match out {
                    mail::Outcome::Resume => frame as *mut TrapContext,
                    mail::Outcome::Park(next) => next,
                };
            }
        }
        // Control 域的臂整个在 `control.rs`（回溯采样 + 一处 `ControlFail`）。
        EnvCall::Control(call) => {
            control::dispatch(frame, call, &ident);
        }
        EnvCall::Pie(call) => {
            if let Some(pie::Outcome::Resume) = pie::dispatch(frame, call, ident) {
                return frame as *mut TrapContext;
            }
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
