use crate::core::work::unit::task::Task;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use env::{PieFail, PieToken};
mod forget;
pub(crate) use forget::{forget, same};
mod accord;
mod allocation;
mod cull;
mod ledger;
mod narrow;
mod need;
mod pie;

pub(crate) use allocation::{HoldTable, SourceAuthorization};
pub(crate) use ledger::Pend;
pub(crate) use need::Need;

#[cfg(debug_assertions)]
pub(crate) use crate::core::work::grant::form_ok;
pub(crate) use pie::{Permission, Pie, PieSnapshot, accede, allows, locate, vestor};

pub(crate) use accord::{accord, clear_heir};
pub(crate) use cull::{doom, release, revoke};
pub(crate) use narrow::reduce;

pub(super) const RETRIES: usize = 8;

pub(crate) fn changed(task: &Task) {
    task.pend.version.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn live(task: &Task) -> bool {
    !task.dead()
}

pub(super) fn with_pair<T>(a: &Task, b: &Task, run: impl FnOnce() -> T) -> T {
    if a.ident.id == b.ident.id {
        let _guard = a.pend.lock();
        let _commit = crate::core::work::commit();
        run()
    } else if a.ident.id.get() < b.ident.id.get() {
        let _first = a.pend.lock();
        let _second = b.pend.lock();
        let _commit = crate::core::work::commit();
        run()
    } else {
        let _second = b.pend.lock();
        let _first = a.pend.lock();
        let _commit = crate::core::work::commit();
        run()
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

pub(super) fn observe(
    tasks: &mut Vec<(Arc<Task>, usize)>,
    task: &Arc<Task>,
    version: usize,
) -> Result<(), PieFail> {
    if tasks.iter().any(|(old, _)| Arc::ptr_eq(old, task)) {
        return Ok(());
    }
    remember(tasks, task, version)
}

pub(crate) fn available(pie: &PieSnapshot) -> Result<(), env::MailFail> {
    if let Some(pole) = pie.mail().pole()
        && pole.reserved() != 0
    {
        return Err(env::MailFail::HandedOver);
    }
    if let Some(heir) = pie.heir()
        && crate::core::work::room::scheduler::roster::muster(heir.task)
            .and_then(|t| t.upgrade())
            .is_some_and(|t| locate(&t, heir.token).is_some())
    {
        return Err(env::MailFail::HandedOver);
    }
    Ok(())
}

pub(super) fn with_tasks<T>(
    tasks: &[(Arc<Task>, usize)],
    run: impl FnOnce() -> T,
) -> Result<T, PieFail> {
    with_tasks_checked(tasks, || false, run)
}

pub(super) fn with_tasks_checked<T>(
    tasks: &[(Arc<Task>, usize)],
    validate: impl FnOnce() -> bool,
    run: impl FnOnce() -> T,
) -> Result<T, PieFail> {
    crate::machine::sync::reserve_depend(tasks.len() + 8).map_err(|_| PieFail::OoM)?;
    debug_assert!(
        tasks
            .windows(2)
            .all(|pair| pair[0].0.ident.id.get() < pair[1].0.ident.id.get())
    );
    let mut guards = Vec::new();
    guards.try_reserve(tasks.len()).map_err(|_| PieFail::OoM)?;
    for (task, _) in tasks {
        guards.push(task.pend.lock());
    }
    let commit = crate::core::work::commit();
    let valid = tasks
        .iter()
        .all(|(task, version)| task.pend.version.load(Ordering::Relaxed) == *version)
        || validate();
    let result = if valid { Ok(run()) } else { Err(PieFail::Busy) };
    drop(commit);
    while guards.pop().is_some() {}
    result
}

pub(super) fn insert_heir(task: &Task, parent: PieToken, child: Weak<Task>, token: PieToken) {
    let mut heirs = task.pend.heirs.lock();
    let at = heirs.partition_point(|(source, _, _)| source.get() <= parent.get());
    heirs.insert(at, (parent, child, token));
}

pub(super) fn remove_heir(task: &Task, parent: PieToken, token: PieToken) {
    let mut heirs = task.pend.heirs.lock();
    let start = heirs.partition_point(|(source, _, _)| source.get() < parent.get());
    if let Some(at) = heirs[start..]
        .iter()
        .take_while(|(source, _, _)| *source == parent)
        .position(|(_, _, child)| *child == token)
    {
        heirs.remove(start + at);
    }
}

pub(crate) fn insert(task: &Task, pie: Pie) -> Result<(), PieFail> {
    let _closed = task.pend.lock();
    let _commit = crate::core::work::commit();
    if !live(task) {
        return Err(PieFail::Dead);
    }
    let mut pies = task.pend.pies.lock();
    pies.try_reserve(1).map_err(|_| PieFail::OoM)?;
    pies.push(pie);
    changed(task);
    Ok(())
}

pub(crate) fn notify(task: env::TaskId, token: PieToken) {
    let _ = crate::core::work::room::messenger::signal(crate::core::work::room::messenger::WakeKey::Inspect {
        task,
        token: token.get(),
    });
}
