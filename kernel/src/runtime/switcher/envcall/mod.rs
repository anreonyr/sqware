use alloc::sync::Arc;

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
pub(crate) mod pie;
mod room;
pub(crate) mod resources;
pub(crate) mod tole;
mod unit;

fn ret_err<E: env::FailCode>(frame: &mut TrapContext, e: E) -> *mut TrapContext {
    frame.gpr.set_x(Gprs::A0, e.code() as usize);
    frame as *mut TrapContext
}

fn instr_len(space: &Space, sepc: KVirt) -> usize {
    let b0 = space.instruction_byte(sepc.as_usize()).unwrap_or(0b11);
    if b0 & 0b11 == 0b11 { 4 } else { 2 }
}

pub fn dispatch(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> Option<*mut TrapContext> {
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
    frame.sepc = KVirt::wrap(
        frame
            .sepc
            .as_usize()
            .checked_add(instr_len(&ident.team.space, frame.sepc))
            .unwrap_or(frame.sepc.as_usize()),
    );
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

    };
    frame as *mut TrapContext
}
