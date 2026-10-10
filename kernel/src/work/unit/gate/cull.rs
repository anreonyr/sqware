use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use env::{PieFail, PieToken, TaskId};

use super::pie::{AnyPie, Heir};
use crate::work::mail::pole;
use crate::work::unit::task::Task;

pub(crate) struct Cleanup {
    removed: Vec<AnyPie>,
    tasks: Vec<TaskId>,
}

impl Cleanup {
    pub(crate) fn finish(self) -> usize {
        let count = self.removed.len();
        for pie in self.removed {
            // Invalidate the exact reference and notify its direct ancestor after
            // gates are released; the ancestor may have regained exclusive use.
            for task in &self.tasks {
                super::notify(*task, pie.token());
            }
            if let (Some(token), Some(parent)) = (pie.sire(), pie.lord().upgrade()) {
                super::notify(parent.ident.id, token);
            }
            if let Some(p) = pie.snapshot().pole() {
                pole::shut(&p, pie.token()).expect("cull: unmap token");
            }
            drop(pie);
        }
        count
    }
}

/// Collect actual transfer descendants, plus the root's parent for unlinking.
/// Each read is under one task gate. The commit validates versions or the
/// actual subtree after locking every collected task.
#[allow(clippy::type_complexity)]
pub(super) fn collect(
    task: &Arc<Task>,
    token: PieToken,
    recursive: bool,
) -> Result<(Vec<(Arc<Task>, usize)>, Vec<(Arc<Task>, PieToken)>), PieFail> {
    let mut tasks = Vec::new();
    let mut nodes = Vec::new();
    nodes.try_reserve(1).map_err(|_| PieFail::OoM)?;
    nodes.push((task.clone(), token));
    let mut cursor = 0;
    while cursor < nodes.len() {
        let (task, token) = nodes[cursor].clone();
        let parent = {
            let _gate = task.gate.lock();
            super::observe(&mut tasks, &task, task.gate.version.load(Ordering::Relaxed))?;
            let pie = super::locate(&task, token).ok_or(PieFail::Busy)?;
            let parent = if cursor == 0 {
                pie.lord().upgrade()
            } else {
                None
            };
            if cursor == 0 || recursive {
                let heirs = task.gate.heirs.lock();
                let start = heirs.partition_point(|(source, _, _)| source.get() < token.get());
                for (_, child, child_token) in heirs[start..]
                    .iter()
                    .take_while(|(source, _, _)| *source == token)
                {
                    if let Some(child) = child.upgrade() {
                        nodes.try_reserve(1).map_err(|_| PieFail::OoM)?;
                        nodes.push((child, *child_token));
                    }
                }
            }
            parent
        };
        if let Some(parent) = parent {
            let _gate = parent.gate.lock();
            super::observe(
                &mut tasks,
                &parent,
                parent.gate.version.load(Ordering::Relaxed),
            )?;
        }
        cursor += 1;
    }
    tasks.sort_unstable_by_key(|(task, _)| task.ident.id.get());
    Ok((tasks, nodes))
}

/// Validate the actual subtree rather than rejecting unrelated task changes.
/// Every holder and the root's current parent must already be locked. A changed
/// relation involving an uncollected task requires a fresh collection.
fn relations_match(tasks: &[(Arc<Task>, usize)], nodes: &[(Arc<Task>, PieToken)]) -> bool {
    for (index, (task, token)) in nodes.iter().enumerate() {
        let Some(pie) = super::locate(task, *token) else {
            return false;
        };
        if let Some(parent) = pie.sire() {
            let Some(lord) = pie.lord().upgrade() else {
                return false;
            };
            if index == 0 {
                if !tasks.iter().any(|(holder, _)| Arc::ptr_eq(holder, &lord)) {
                    return false;
                }
            } else if !nodes
                .iter()
                .any(|(holder, source)| *source == parent && Arc::ptr_eq(holder, &lord))
            {
                return false;
            }
            if !lord
                .gate
                .heirs
                .lock()
                .iter()
                .any(|(source, child, child_token)| {
                    *source == parent
                        && *child_token == *token
                        && child.ptr_eq(&Arc::downgrade(task))
                })
            {
                return false;
            }
        } else if index != 0 {
            return false;
        }
        let heirs = task.gate.heirs.lock();
        let start = heirs.partition_point(|(source, _, _)| source.get() < token.get());
        for (_, child, child_token) in heirs[start..]
            .iter()
            .take_while(|(source, _, _)| *source == *token)
        {
            if child.strong_count() != 0
                && !nodes.iter().skip(1).any(|(holder, token)| {
                    *token == *child_token && child.ptr_eq(&Arc::downgrade(holder))
                })
            {
                return false;
            }
        }
    }
    true
}

/// The root and all relevant tasks are locked; no allocation after this point.
pub(super) fn take(task: &Task, token: PieToken) -> Option<AnyPie> {
    let pie = {
        let mut pies = task.gate.pies.lock();
        let at = pies.iter().position(|p| p.token() == token)?;
        pies.remove(at)
    };
    pie.invalidate();
    if let (Some(parent), Some(lord)) = (pie.sire(), pie.lord().upgrade()) {
        super::remove_heir(&lord, parent, token);
        super::accord::clear_heir_locked(
            &lord,
            parent,
            Heir {
                task: task.ident.id,
                token,
            },
        );
        super::changed(&lord);
    }
    task.gate
        .heirs
        .lock()
        .retain(|(parent, _, _)| *parent != token);
    super::changed(task);
    Some(pie)
}

pub(super) fn cull(
    task: &Arc<Task>,
    token: PieToken,
    caller: Option<&Arc<Task>>,
    closing: bool,
) -> Result<Cleanup, PieFail> {
    for _ in 0..super::RETRIES {
        if super::locate(task, token).is_none() {
            return Err(PieFail::Denied);
        }
        let (mut tasks, nodes) = match collect(task, token, true) {
            Err(PieFail::Busy) => continue,
            result => result?,
        };
        if let Some(caller) = caller {
            let _gate = caller.gate.lock();
            super::observe(
                &mut tasks,
                caller,
                caller.gate.version.load(Ordering::Relaxed),
            )?;
        }
        tasks.sort_unstable_by_key(|(task, _)| task.ident.id.get());
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        removed.try_reserve(nodes.len()).map_err(|_| PieFail::OoM)?;
        changed.try_reserve(tasks.len()).map_err(|_| PieFail::OoM)?;
        let result = super::with_tasks_checked(
            &tasks,
            || relations_match(&tasks, &nodes),
            || {
                let root = super::locate(task, token).ok_or(PieFail::Denied)?;
                if let Some(caller) = caller {
                    let parent = root.sire().ok_or(PieFail::Denied)?;
                    let lord = root.lord().upgrade().ok_or(PieFail::Denied)?;
                    if !Arc::ptr_eq(&lord, caller) || super::locate(caller, parent).is_none() {
                        return Err(PieFail::Denied);
                    }
                }
                // Exit must withdraw authority even if mapping/construction is busy.
                // Normal release/revoke retain their existing Busy checks.
                let pole = root.pole();
                let _operation = if !closing && let Some(p) = &pole {
                    let operation = p.backing().operation().ok_or(PieFail::Busy)?;
                    if p.backing().reserved() != 0 {
                        return Err(PieFail::Busy);
                    }
                    Some(operation)
                } else {
                    None
                };
                // Descendants first, so each parent still exists while unlinking.
                for (holder, token) in nodes.iter().rev() {
                    if let Some(pie) = take(holder, *token) {
                        removed.push(pie);
                    }
                }
                for (holder, _) in &tasks {
                    changed.push(holder.ident.id);
                }
                Ok(())
            },
        );
        match result {
            Err(PieFail::Busy) => {
                core::hint::spin_loop();
                continue;
            }
            result => result??,
        }
        return Ok(Cleanup {
            removed,
            tasks: changed,
        });
    }
    Err(PieFail::Busy)
}

/// Close admission before collecting. Resource sealing still runs even when
/// allocating the removal list fails, so a dead owner cannot retain authority.
pub(crate) fn doom(task: &Arc<Task>) {
    let tokens = {
        let _gate = task.gate.lock();
        super::changed(task);
        let mut tokens = Vec::new();
        let pies = task.gate.pies.lock();
        if tokens.try_reserve(pies.len()).is_ok() {
            tokens.extend(pies.iter().map(|p| p.token()));
        }
        tokens
    };
    seal_owned(task);
    for token in tokens {
        loop {
            match cull(task, token, None, true) {
                Ok(cleanup) => {
                    cleanup.finish();
                    break;
                }
                // No syscall caller can retry an exit hook. Do not abandon
                // borrowed descendants merely because a version changed.
                Err(PieFail::Busy) => core::hint::spin_loop(),
                Err(_) => break,
            }
        }
    }
}

fn seal_owned(task: &Task) {
    if task.ident.id.get() == 0 {
        return;
    }
    let mut after = 0;
    loop {
        let pie = {
            let _gate = task.gate.lock();
            task.gate
                .pies
                .lock()
                .iter()
                .filter(|pie| pie.owner_task() == task.ident.id)
                .filter(|pie| pie.token().get() > after)
                .min_by_key(|pie| pie.token().get())
                .map(|p| p.snapshot())
        };
        let Some(pie) = pie else {
            break;
        };
        after = pie.token().get();
        pie.seal();
    }
}
