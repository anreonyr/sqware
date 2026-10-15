use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::Access;
use crate::core::work::unit::space::Space;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::{Gprs, TrapContext};
use crate::putln;
use env::DebugFail;

use super::outcome::ret_err;

pub static TRACE: ::core::sync::atomic::AtomicBool = ::core::sync::atomic::AtomicBool::new(false);

const DBCN_MAX: usize = 256;

pub(super) fn put(frame: &mut TrapContext, ident: &TaskIdent, buf: usize, len: usize) {
    if len == 0 {
        ret_err(frame, DebugFail::Denied);
        return;
    }
    let n = len.min(DBCN_MAX);
    let mut text = [0u8; DBCN_MAX];
    let space = &ident.team.space;
    if !Space::user_range(KVirt::wrap(buf), len) || !space.copy_in(&mut text[..n], KVirt::wrap(buf))
    {
        ret_err(frame, DebugFail::Denied);
        return;
    }
    let text = ::core::str::from_utf8(&text[..n]).unwrap_or("<non-utf8>");
    putln!("{text}");
    frame.gpr.set_x(Gprs::A0, n);
}

pub(super) fn get(frame: &mut TrapContext, ident: &TaskIdent, buf: usize, len: usize) {
    if len == 0 || len > DBCN_MAX {
        ret_err(frame, DebugFail::Denied);
        return;
    }
    if !ident
        .team
        .space
        .validate_access(KVirt::wrap(buf), len, Access::WRITE)
    {
        ret_err(frame, DebugFail::Denied);
        return;
    }
    let mut stage = [0u8; DBCN_MAX];
    let n = match crate::machine::console::read(&mut stage[..len]) {
        Some(n) if n <= len => n,
        _ => {
            ret_err(frame, DebugFail::Denied);
            return;
        }
    };
    if n == 0 {
        frame.gpr.set_x(Gprs::A0, 0);
        return;
    }
    let space = &ident.team.space;
    if !space.copy_out(&stage[..n], KVirt::wrap(buf)) {
        ret_err(frame, DebugFail::Denied);
        return;
    }
    frame.gpr.set_x(Gprs::A0, n);
}

pub(super) fn set_trace(on: usize) -> usize {
    let v = on != 0;
    TRACE.store(v, ::core::sync::atomic::Ordering::Relaxed);
    v as usize
}
