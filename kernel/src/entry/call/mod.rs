use alloc::sync::Arc;

use env::{DebugCall, EnvCall};

use crate::core::work::record::trace::{self, EnvEvent, EventKind};
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::{Gprs, TrapContext};

mod chrono;
mod control;
mod debug;
pub(crate) mod mail;
pub(in crate::entry) mod memory;
mod outcome;
pub(crate) mod pie;
pub(crate) mod resources;
mod room;
pub(crate) mod tole;
mod unit;

use outcome::{Outcome, ret_err};

/// 把这一次调用做掉，交回要接着跑的那一帧。
///
/// **不碰 `sepc`**：客人那条 `ebreak` 有多宽、之后从哪里续跑，是入口那一手的事
/// （`entry::trap::envcall`）——这里只管把这一格调用服务完，并把答案写进这一帧。
pub fn dispatch(frame: &mut TrapContext, ident: Arc<TaskIdent>) -> Option<*mut TrapContext> {
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
    let envcall = match EnvCall::from_wire(number, &regs) {
        Ok(c) => c,
        Err(_) => {
            ret_err(frame, env::DispatchFail::Unknown);
            return Some(frame as *mut TrapContext);
        }
    };
    let outcome = match envcall {
        EnvCall::Room(call) => room::dispatch(frame, call, ident),
        EnvCall::Unit(call) => unit::dispatch(frame, call, ident),
        EnvCall::Memory(call) => {
            memory::dispatch(frame, call, &ident);
            Outcome::Resume
        }
        EnvCall::Chrono(call) => {
            chrono::dispatch(frame, call);
            Outcome::Resume
        }
        EnvCall::Mail(call) => mail::dispatch(frame, call, ident),
        EnvCall::Control(call) => {
            control::dispatch(frame, call, &ident);
            Outcome::Resume
        }
        EnvCall::Pie(call) => {
            pie::dispatch(frame, call, ident);
            Outcome::Resume
        }
        EnvCall::Debug(DebugCall::Put { buf, len }) => {
            debug::put(frame, &ident, buf.get(), len);
            Outcome::Resume
        }
        EnvCall::Debug(DebugCall::Get { buf, len }) => {
            debug::get(frame, &ident, buf.get(), len);
            Outcome::Resume
        }
        EnvCall::Debug(DebugCall::SetTrace { on }) => {
            frame.gpr.set_x(Gprs::A0, debug::set_trace(on));
            Outcome::Resume
        }
    };
    outcome.next(frame as *mut TrapContext)
}
