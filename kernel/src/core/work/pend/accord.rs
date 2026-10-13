use alloc::sync::{Arc, Weak};

use env::{Mark, PieFail, PieToken};

use super::pie::{Heir, Need, Permission};
use crate::core::work::grant::narrows;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::task::Task;

pub(crate) fn accord(
    caller: &Arc<Task>,
    src: PieToken,
    dst: &Weak<Task>,
    subset: Permission,
    mark: Mark,
) -> Result<usize, PieFail> {
    let target = dst.upgrade().ok_or(PieFail::Denied)?;
    let token = super::with_pair(caller, &target, || {
        if !super::live(caller) || !super::live(&target) {
            return Err(PieFail::Dead);
        }
        let pie = super::locate(caller, src).ok_or(PieFail::Denied)?;
        let pole = pie.mail().pole();
        let _operation = if let Some(p) = &pole {
            let operation = p.operation().ok_or(PieFail::Busy)?;
            if p.reserved() != 0 {
                return Err(PieFail::HandedOver);
            }
            Some(operation)
        } else {
            None
        };
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if !pie.allows(Need::Vest) || !narrows(pie.permission(), subset) {
            return Err(PieFail::Denied);
        }
        if pie.heir().is_some() {
            return Err(PieFail::HandedOver);
        }
        if pie.mail().tole().is_some_and(|p| p.has_cells()) {
            return Err(PieFail::Denied);
        }
        let mark = if mark == Mark::NONE { pie.mark() } else { mark };
        let mut granted = pie.vest(mark, subset)?;
        granted.parent(src, Arc::downgrade(caller));
        caller
            .pend
            .heirs
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        target
            .pend
            .pies
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        let token = granted.token();
        super::insert_heir(caller, src, Arc::downgrade(&target), token);
        if pie.permission().contains(Permission::ONLY) {
            let mut pies = caller.pend.pies.lock();
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
        target.pend.pies.lock().push(granted);
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
    let pend_guard = task.pend.lock();
    let commit_guard = crate::core::work::unit::commit();
    let child = {
        let heirs = task.pend.heirs.lock();
        heirs
            .iter()
            .find(|(parent, _, child)| *parent == token && *child == expected.token)
            .and_then(|(_, task, _)| task.upgrade())
    };
    if child.is_some_and(|child| super::locate(&child, expected.token).is_some()) {
        return false;
    }
    let cleared = clear_heir_locked(task, token, expected);
    if cleared {
        super::remove_heir(task, token, expected.token);
    }
    drop(commit_guard);
    drop(pend_guard);
    if cleared {
        super::notify(task.ident.id, token);
    }
    cleared
}

pub(super) fn clear_heir_locked(task: &Task, token: PieToken, expected: Heir) -> bool {
    let mut pies = task.pend.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    if pie.heir() != Some(&expected) {
        return false;
    }
    pie.set_heir(None);
    super::changed(task);
    true
}
