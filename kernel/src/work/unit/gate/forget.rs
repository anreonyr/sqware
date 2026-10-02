use super::AnyPie;
use crate::work::unit::task::Task;
use alloc::sync::Arc;
use env::{Permission, PieFail, PieToken};

pub(crate) fn forget(task: &Arc<Task>, token: PieToken) -> Result<(), PieFail> {
    let _graph = super::GRAPH.lock();
    let parent = {
        let pies = task.pies.lock();
        let pie = pies
            .iter()
            .find(|p| p.token() == token)
            .ok_or(PieFail::Denied)?;
        if pie.permission().contains(Permission::ONLY) {
            return Err(PieFail::Denied);
        }
        pie.sire().ok_or(PieFail::Denied)?
    };
    // The snapshot and edge rewrite are inside the same graph transaction as Accord.
    // No new direct child can appear between this scan and removal of the parent.
    let snapshot = super::snap();
    if snapshot.is_empty() {
        return Err(PieFail::OoM);
    }
    for weak in snapshot {
        let Some(holder) = weak.upgrade() else {
            continue;
        };
        let mut pies = holder.pies.lock();
        for pie in pies.iter_mut() {
            if pie.sire() != Some(token) {
                continue;
            }
            match pie {
                AnyPie::Hole(p) => p.sire = Some(parent),
                AnyPie::Pole(p) => p.sire = Some(parent),
                AnyPie::Nole(p) => p.sire = Some(parent),
                AnyPie::Tole(p) => p.sire = Some(parent),
            }
        }
    }
    let mut pies = task.pies.lock();
    let at = pies
        .iter()
        .position(|p| p.token() == token)
        .ok_or(PieFail::Denied)?;
    let pie = pies.remove(at);
    drop(pies);
    if let AnyPie::Pole(p) = pie {
        let _ = crate::work::mail::pole::shut(p.meta(), token);
    }
    Ok(())
}

pub(crate) fn same(task: &Arc<Task>, a: PieToken, b: PieToken) -> Result<bool, PieFail> {
    let pies = task.pies.lock();
    let find = |token| {
        pies.iter()
            .find(|p| p.token() == token)
            .ok_or(PieFail::Denied)
    };
    let (a, b) = (find(a)?, find(b)?);
    if !a.alive() || !b.alive() { return Err(PieFail::Dead); }
    Ok(match (a, b) {
        (AnyPie::Hole(a), AnyPie::Hole(b)) => Arc::ptr_eq(a.meta(), b.meta()),
        (AnyPie::Pole(a), AnyPie::Pole(b)) => Arc::ptr_eq(a.meta(), b.meta()),
        (AnyPie::Nole(a), AnyPie::Nole(b)) => Arc::ptr_eq(a.meta(), b.meta()),
        (AnyPie::Tole(a), AnyPie::Tole(b)) => Arc::ptr_eq(a.meta(), b.meta()),
        _ => false,
    })
}
