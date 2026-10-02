use crate::putln;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::unit::task::TaskIdent;
use env::DebugFail;

pub static TRACE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

const DBCN_MAX: usize = 256;

pub(super) fn put(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    buf: usize,
    len: usize,
) -> *mut TrapContext {
    if len == 0 {
        return err(frame, DebugFail::Denied);
    }
    let n = len.min(DBCN_MAX);
    let mut text = [0u8; DBCN_MAX];
    let space = &ident.team.space;
    if !crate::work::unit::space::Space::user_range(buf, len) || !space.copy_in(&mut text[..n], buf)
    {
        return err(frame, DebugFail::Denied);
    }
    let text = core::str::from_utf8(&text[..n]).unwrap_or("<non-utf8>");
    putln!("{text}");
    frame.gpr.set_x(Gprs::A0, n);
    frame
}

pub(super) fn get(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    buf: usize,
    len: usize,
) -> *mut TrapContext {
    if len == 0 || len > DBCN_MAX {
        return err(frame, DebugFail::Denied);
    }
    if !ident.team.space.validate_write(buf, len) {
        return err(frame, DebugFail::Denied);
    }
    let mut stage = [0u8; DBCN_MAX];
    let n = match crate::console::read(&mut stage[..len]) {
        Some(n) if n <= len => n,
        _ => return err(frame, DebugFail::Denied),
    };
    if n == 0 {
        frame.gpr.set_x(Gprs::A0, 0);
        return frame;
    }
    let space = &ident.team.space;
    if !crate::work::mail::copy_out(space, &stage[..n], buf) {
        return err(frame, DebugFail::Denied);
    }
    frame.gpr.set_x(Gprs::A0, n);
    frame
}

pub(super) fn set_trace(on: usize) -> usize {
    let v = on != 0;
    TRACE.store(v, core::sync::atomic::Ordering::Relaxed);
    v as usize
}

fn err(frame: &mut TrapContext, e: DebugFail) -> *mut TrapContext {
    frame.gpr.set_x(Gprs::A0, e.code() as usize);
    frame
}
