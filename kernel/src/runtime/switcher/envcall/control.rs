use alloc::sync::Arc;

use env::{ControlCall, ControlFail};

use crate::runtime::diagnose::frame::{self, ResolveCfg, StackReader};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::task::TaskIdent;

use super::ret_err;

pub(super) fn dispatch(frame: &mut TrapContext, call: ControlCall, ident: &Arc<TaskIdent>) {
    match call {
        ControlCall::Backtrace { buf, frames } => {
            let world = ident.team.space.kind();
            let sp = frame.gpr.x(Gprs::SP);
            let fp = frame.gpr.x(Gprs::S0);
            let mut reader = StackReader::new(frame.user_satp.ppn());
            let cfg = ResolveCfg::normal(world, sp.saturating_add(frame::SPAN));
            let code = move |_w: usize| true;
            let (pc_arr, count) = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
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
            if ok {
                frame.gpr.set_x(Gprs::A0, keep);
            } else {
                ret_err(frame, ControlFail::Denied);
            }
        }
    }
}