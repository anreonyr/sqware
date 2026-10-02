use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use env::{DebugCall, EnvCall};

use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::diagnose::trace::{self, EnvEvent, EventKind};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::space::Space;
use crate::work::unit::task::TaskIdent;

mod chrono;
mod control;
mod debug;
pub(crate) mod mail;
mod memory;
mod pie;
mod room;
mod tole;
mod unit;

fn ret_err<E: env::FailCode>(frame: &mut TrapContext, e: E) -> *mut TrapContext {
    frame.gpr.set_x(Gprs::A0, e.code() as usize);
    frame as *mut TrapContext
}

fn instr_len(space: &Space, sepc: KVirt) -> usize {
    let b0 = space
        .translate(sepc)
        .map(|(pa, _)| unsafe { core::ptr::read_volatile(pa.as_usize() as *const u8) })
        .unwrap_or(0b11);
    if b0 & 0b11 == 0b11 { 4 } else { 2 }
}

/// **心跳（临时诊断）**：每颗 hart 每 [`HB_EVERY`] 次系统调用打一行 `hb hart=… n=…`。
///
/// **为什么住这一格**：系统调用那一刻走的是**任务上下文**（内核栈），而**陷阱上下文里写控制台
/// 是静默的**——`console::write_str` 对既不在 identity 段、又译不出的那段缓冲**一个字都不写**
/// （量过：把心跳打在定时器那一路、每 100 拍一行，整整一跑零行）。这一条给的是
/// "**这颗 hart 还在替任务办事**"：卡住那一族（整机跑完不出场）现场零读数、空闲看门狗也不响
/// ⇒ 有 hart 一直在跑；哪几颗的心跳从此断掉，就是哪几颗卡住了。
const HB_EVERY: u64 = 5_000;

static HB: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

pub fn dispatch(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> Option<*mut TrapContext> {
    let h = crate::hart::hart_id().get();
    if h < HB.len() {
        let mine = HB[h].fetch_add(1, Ordering::Relaxed) + 1;
        if mine % HB_EVERY == 0 {
            crate::putln!(
                "hb hart={} n={} task={} call={}",
                h,
                mine,
                ident.id.get(),
                frame.gpr.x(Gprs::A7)
            );
        }
    }
    let pa = dispatch_inner(frame, ident);
    if pa.is_null() { None } else { Some(pa) }
}

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
    let envcall = match EnvCall::from_wire(number, &regs) {
        Ok(c) => c,
        Err(_) => return ret_err(frame, env::DispatchFail::Unknown),
    };
    match envcall {
        EnvCall::Room(call) => match room::dispatch(frame, call, ident) {
            room::Outcome::Resume => {}
            room::Outcome::Switch(pa) => return pa,
            room::Outcome::Exit => return core::ptr::null_mut(),
        },
        EnvCall::Unit(call) => match unit::dispatch(frame, call, ident) {
            unit::Outcome::Resume => {}
            unit::Outcome::Switch(pa) => return pa,
        },
        EnvCall::Memory(call) => {
            memory::dispatch(frame, call, &ident);
        }
        EnvCall::Chrono(call) => chrono::dispatch(frame, call),
        EnvCall::Mail(call) => {
            if let Some(out) = mail::dispatch(frame, call, ident) {
                return match out {
                    mail::Outcome::Resume => frame as *mut TrapContext,
                    mail::Outcome::Park(next) => next,
                };
            }
        }
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
