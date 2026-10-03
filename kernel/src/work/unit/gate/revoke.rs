use alloc::sync::Weak;

use env::{PieFail, PieToken};

use super::cull;
use crate::work::unit::task::Task;

pub(crate) fn revoke(
    caller: &Task,
    target: &Weak<Task>,
    token: PieToken,
) -> Result<usize, PieFail> {
    let graph = super::GRAPH.lock();
    let target = target.upgrade().ok_or(PieFail::Denied)?;
    let source_token = super::locate(&target, token).and_then(|pie| pie.sire()).ok_or(PieFail::Denied)?;
    let source = super::locate(caller, source_token).ok_or(PieFail::Denied)?;
    let _operation = if let super::AnyPie::Pole(p) = &source {
        let operation = p.meta().backing().operation().ok_or(PieFail::Busy)?;
        if p.meta().backing().reserved() != 0 { return Err(PieFail::Busy); }
        Some(operation)
    } else { None };
    let snap = super::snap();
    let sire = {
        let pies = target.pies.lock();
        let pie = pies
            .iter()
            .find(|p| p.token() == token)
            .ok_or(PieFail::Denied)?;
        pie.sire().ok_or(PieFail::Denied)?
    };
    let mine = {
        let pies = caller.pies.lock();
        pies.iter().any(|p| p.token() == sire)
    };
    if !mine {
        return Err(PieFail::Denied);
    }
    let cleanup = cull::cull((target, token), &snap);
    drop(graph);
    Ok(cleanup.finish())
}
