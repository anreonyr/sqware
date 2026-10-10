use alloc::sync::{Arc, Weak};

use env::{Mark, PieFail, PieToken};

use super::pie::{Heir, Need, Permission};
use crate::work::room::messenger::{self, WakeKey};
use crate::work::unit::task::Task;

pub(crate) fn accord(
    caller: &Arc<Task>,
    src: PieToken,
    dst: &Weak<Task>,
    subset: Permission,
    mark: Mark,
) -> Result<usize, PieFail> {
    let target = dst.upgrade().ok_or(PieFail::Denied)?;
    let token = super::with_pair(caller, &target, |source_closed, target_closed| {
        if !super::live(caller, source_closed) || !super::live(&target, target_closed) {
            return Err(PieFail::Dead);
        }
        let pie = super::locate(caller, src).ok_or(PieFail::Denied)?;
        let pole = pie.pole();
        let _operation = if let Some(p) = &pole {
            let operation = p.backing().operation().ok_or(PieFail::Busy)?;
            if p.backing().reserved() != 0 {
                return Err(PieFail::HandedOver);
            }
            Some(operation)
        } else {
            None
        };
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if !pie.allows(Need::Grant)
            || !pie.covers(subset)
            || !super::pie::form_ok(pie.permission(), subset)
        {
            return Err(PieFail::Denied);
        }
        if pie.heir().is_some() {
            return Err(PieFail::HandedOver);
        }
        if pie.tole().is_some_and(|p| p.has_cells()) {
            return Err(PieFail::Denied);
        }
        let badge = if mark == Mark::NONE { pie.mark() } else { mark };
        let mut granted = pie.grant(badge, subset)?;
        granted.parent(src, Arc::downgrade(caller));
        // Reserve both records before changing either side, including ONLY's heir.
        caller
            .gate.heirs
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        target
            .gate.pies
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        let token = granted.token();
        super::insert_heir(caller, src, Arc::downgrade(&target), token);
        if pie.permission().contains(Permission::ONLY) {
            let mut pies = caller.gate.pies.lock();
            let source = pies
                .iter_mut()
                .find(|p| p.token() == src)
                .expect("locked source");
            let heir = Heir {
                task: target.ident.id,
                token,
            };
            source.set_heir(Some(heir));
        }
        // Block the exclusive source before publishing the recipient's token.
        target.gate.pies.lock().push(granted);
        super::changed(caller);
        if !Arc::ptr_eq(caller, &target) {
            super::changed(&target);
        }
        Ok(token)
    })?;
    let _ = messenger::wake(
        WakeKey::Pies {
            task: target.ident.id,
        },
        &target.life(),
    );
    super::notify(caller.ident.id, src);
    super::notify(target.ident.id, token);
    Ok(token.get())
}

pub(crate) fn clear_heir(task: &Task, token: PieToken, expected: Heir) -> bool {
    let gate_guard = task.gate.lock();
    let commit_guard = super::super::commit();
    let child = {
        let heirs = task.gate.heirs.lock();
        heirs
            .iter()
            .find(|(parent, _, child)| *parent == token && *child == expected.token)
            .and_then(|(_, task, _)| task.upgrade())
    };
    // Recheck after acquiring the source gate: usable() may have observed the
    // source heir before accord finished publishing the child.
    if child.is_some_and(|child| super::locate(&child, expected.token).is_some()) {
        return false;
    }
    let cleared = clear_heir_locked(task, token, expected);
    if cleared {
        super::remove_heir(task, token, expected.token);
    }
    drop(commit_guard); drop(gate_guard);
    if cleared { super::notify(task.ident.id, token); }
    cleared
}

pub(super) fn clear_heir_locked(task: &Task, token: PieToken, expected: Heir) -> bool {
    let mut pies = task.gate.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    // A stale usable() observation must not clear a newer exclusive transfer.
    if pie.heir() != Some(&expected) {
        return false;
    }
    pie.set_heir(None);
    super::changed(task);
    true
}
