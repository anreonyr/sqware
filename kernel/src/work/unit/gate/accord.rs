use alloc::sync::{Arc, Weak};

use env::{Mark, PieFail, PieToken};

use super::pie::{AnyPie, Heir, Need, Permission, new_pie};
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
        let _operation = if let AnyPie::Pole(p) = &pie {
            let operation = p.meta().backing().operation().ok_or(PieFail::Busy)?;
            if p.meta().backing().reserved() != 0 {
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
        if let AnyPie::Tole(p) = &pie
            && p.meta().has_subs()
        {
            return Err(PieFail::Denied);
        }
        let badge = if mark == Mark::NONE { pie.mark() } else { mark };
        let mut granted = match &pie {
            AnyPie::Hole(p) => AnyPie::Hole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Pole(p) => AnyPie::Pole(super::try_new_pie(
                p.meta().clone(),
                badge,
                subset,
                Some(src),
            )?),
            AnyPie::Nole(p) => AnyPie::Nole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Tole(p) => AnyPie::Tole(new_pie(p.meta().clone(), badge, subset, Some(src))),
        };
        granted.parent(src, Arc::downgrade(caller));
        // Reserve both records before changing either side, including ONLY's heir.
        caller
            .heirs
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        target
            .pies
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        let token = granted.token();
        super::insert_heir(caller, src, Arc::downgrade(&target), token);
        if pie.permission().contains(Permission::ONLY) {
            let mut pies = caller.pies.lock();
            let source = pies
                .iter_mut()
                .find(|p| p.token() == src)
                .expect("locked source");
            let heir = Heir {
                task: target.ident.id,
                token,
            };
            match source {
                AnyPie::Hole(p) => p.heir = Some(heir),
                AnyPie::Pole(p) => p.heir = Some(heir),
                AnyPie::Nole(p) => p.heir = Some(heir),
                AnyPie::Tole(p) => p.heir = Some(heir),
            }
        }
        // Block the exclusive source before publishing the recipient's token.
        target.pies.lock().push(granted);
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
    for task in [caller.ident.id, target.ident.id] {
        let _ = messenger::signal(WakeKey::Capabilities { task });
    }
    Ok(token.get())
}

pub(crate) fn clear_heir(task: &Task, token: PieToken, expected: Heir) -> bool {
    let _gate = task.gate.lock();
    let child = {
        let heirs = task.heirs.lock();
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
    cleared
}

pub(super) fn clear_heir_locked(task: &Task, token: PieToken, expected: Heir) -> bool {
    let mut pies = task.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    // A stale usable() observation must not clear a newer exclusive transfer.
    if pie.heir() != Some(&expected) {
        return false;
    }
    match pie {
        AnyPie::Hole(p) => p.heir = None,
        AnyPie::Pole(p) => p.heir = None,
        AnyPie::Nole(p) => p.heir = None,
        AnyPie::Tole(p) => p.heir = None,
    }
    super::changed(task);
    true
}
