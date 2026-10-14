use alloc::sync::Arc;

use env::{ControlCall, ControlFail};

use crate::entry::report::backtrace::{self, Backtrace, StackReader, WalkConfig};
use crate::machine::switcher::context::{Gprs, TrapContext};
use crate::core::work::unit::task::TaskIdent;

use super::outcome::ret_err;

pub(super) fn dispatch(frame: &mut TrapContext, call: ControlCall, ident: &Arc<TaskIdent>) {
    match call {
        ControlCall::Backtrace { buf, frames } => {
            let world = ident.team.space.kind();
            let sp = frame.gpr.x(Gprs::SP);
            let fp = frame.gpr.x(Gprs::S0);
            let Some(len) = frames.checked_mul(size_of::<usize>()) else {
                ret_err(frame, ControlFail::Denied);
                return;
            };
            if !ident.team.space.memory.validate_write(buf, len)
                || !crate::core::memory::manager::Memory::user_range(sp, backtrace::SPAN)
            {
                ret_err(frame, ControlFail::Denied);
                return;
            }
            let mut reader = StackReader::user(ident.team.space.clone());
            let cfg = WalkConfig::normal(world, sp.saturating_add(backtrace::SPAN));
            let code = move |_w: usize| true;
            let trace = Backtrace::walk(&mut reader, &cfg, sp, fp, Some(&code));
            let pc_arr = trace.frames();
            let keep = pc_arr.len().min(frames);
            let mut bytes = [0u8; backtrace::DEPTH * ::core::mem::size_of::<usize>()];
            for i in 0..keep {
                bytes[i * ::core::mem::size_of::<usize>()..][..::core::mem::size_of::<usize>()]
                    .copy_from_slice(&pc_arr[i].pc.as_usize().to_le_bytes());
            }
            let ok = ident
                .team
                .space
                .memory
                .copy_out(&bytes[..keep * ::core::mem::size_of::<usize>()], buf);
            if ok {
                frame.gpr.set_x(Gprs::A0, keep);
            } else {
                ret_err(frame, ControlFail::Denied);
            }
        }
    }
}
