use crate::work::unit::task::{Task, TaskTag};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use env::{PieFail, PieToken};
mod forget;
pub(crate) use forget::{forget, same};
mod accord;
mod cull;
mod fail;
mod narrow;
mod pie;
mod release;
mod revoke;

pub(crate) use fail::GateFail;
mod snap;

#[cfg(debug_assertions)]
pub(crate) use pie::form_ok;
pub(crate) use pie::{
    AnyPie, Hole, Need, Nole, Permission, Pie, Pole, Tole, accede, allows, locate, new_pie,
    try_new_pie,
};

pub(crate) use accord::{accord, clear_heir};
pub(crate) use cull::doom;
#[cfg(debug_assertions)]
pub(crate) use narrow::narrow;
pub(crate) use narrow::reduce;
pub(crate) use release::release;
pub(crate) use revoke::revoke;
pub(crate) use snap::vestor;

pub(super) const RETRIES: usize = 8;

pub(crate) fn changed(task: &Task) {
    task.version.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn live(task: &Task, closed: bool) -> bool {
    !closed && !matches!(task.tag(), TaskTag::Doomed | TaskTag::Reaped)
}

/// Task gates are always acquired by TaskId; self-transfer acquires only once.
pub(super) fn with_pair<T>(a: &Task, b: &Task, run: impl FnOnce(bool, bool) -> T) -> T {
    if a.ident.id == b.ident.id {
        let guard = a.gate.lock();
        run(*guard, *guard)
    } else if a.ident.id.get() < b.ident.id.get() {
        let first = a.gate.lock();
        let second = b.gate.lock();
        run(*first, *second)
    } else {
        let second = b.gate.lock();
        let first = a.gate.lock();
        run(*first, *second)
    }
}

pub(super) fn remember(
    tasks: &mut Vec<(Arc<Task>, usize)>,
    task: &Arc<Task>,
    version: usize,
) -> Result<(), PieFail> {
    if let Some((_, old)) = tasks.iter().find(|(old, _)| Arc::ptr_eq(old, task)) {
        return if *old == version {
            Ok(())
        } else {
            Err(PieFail::Busy)
        };
    }
    tasks.try_reserve(1).map_err(|_| PieFail::OoM)?;
    tasks.push((task.clone(), version));
    Ok(())
}

/// The caller sorts tasks first. No mutation occurs unless every version matches.
/// Guards must be released in reverse order to restore interrupts last.
pub(super) fn with_tasks<T>(
    tasks: &[(Arc<Task>, usize)],
    run: impl FnOnce() -> T,
) -> Result<T, PieFail> {
    crate::lock::reserve_depend(tasks.len() + 8).map_err(|_| PieFail::OoM)?;
    debug_assert!(
        tasks
            .windows(2)
            .all(|pair| pair[0].0.ident.id.get() < pair[1].0.ident.id.get())
    );
    let mut guards = Vec::new();
    guards.try_reserve(tasks.len()).map_err(|_| PieFail::OoM)?;
    for (task, _) in tasks {
        guards.push(task.gate.lock());
    }
    let valid = tasks
        .iter()
        .all(|(task, version)| task.version.load(Ordering::Relaxed) == *version);
    let result = if valid { Ok(run()) } else { Err(PieFail::Busy) };
    while guards.pop().is_some() {}
    result
}

pub(super) fn insert_heir(task: &Task, parent: PieToken, child: Weak<Task>, token: PieToken) {
    let mut heirs = task.heirs.lock();
    let at = heirs.partition_point(|(source, _, _)| source.get() <= parent.get());
    heirs.insert(at, (parent, child, token));
}

pub(super) fn remove_heir(task: &Task, parent: PieToken, token: PieToken) {
    let mut heirs = task.heirs.lock();
    let start = heirs.partition_point(|(source, _, _)| source.get() < parent.get());
    if let Some(at) = heirs[start..]
        .iter()
        .take_while(|(source, _, _)| *source == parent)
        .position(|(_, _, child)| *child == token)
    {
        heirs.remove(start + at);
    }
}

pub(crate) fn insert(task: &Task, pie: AnyPie) -> Result<(), PieFail> {
    let closed = task.gate.lock();
    if !live(task, *closed) {
        return Err(PieFail::Dead);
    }
    let mut pies = task.pies.lock();
    pies.try_reserve(1).map_err(|_| PieFail::OoM)?;
    pies.push(pie);
    changed(task);
    Ok(())
}
