use alloc::sync::Arc;

use crate::work::unit::task::Task;
use env::{Permission, PieFail, PieToken};

pub(crate) fn forget(task: &Arc<Task>, token: PieToken) -> Result<(), PieFail> {
    for _ in 0..super::RETRIES {
        if super::locate(task, token).is_none() {
            return Err(PieFail::Denied);
        }
        let (tasks, nodes) = match super::cull::collect(task, token, false) {
            Err(PieFail::Busy) => continue,
            result => result?,
        };
        let result = super::with_tasks(&tasks, || {
            let pie = super::locate(task, token).ok_or(PieFail::Denied)?;
            if pie.permission().contains(Permission::ONLY) {
                return Err(PieFail::Denied);
            }
            let parent = pie.sire().ok_or(PieFail::Denied)?;
            let lord = pie.lord().upgrade().ok_or(PieFail::Denied)?;
            if super::locate(&lord, parent).is_none() {
                return Err(PieFail::Denied);
            }
            lord.gate.heirs
                .lock()
                .try_reserve(nodes.len() - 1)
                .map_err(|_| PieFail::OoM)?;
            for (child, child_token) in nodes.iter().skip(1) {
                let mut pies = child.gate.pies.lock();
                let pie = pies
                    .iter_mut()
                    .find(|p| p.token() == *child_token)
                    .expect("locked child");
                pie.parent(parent, Arc::downgrade(&lord));
                drop(pies);
                super::insert_heir(&lord, parent, Arc::downgrade(child), *child_token);
                super::changed(child);
            }
            let removed = super::cull::take(task, token).expect("locked forgotten token");
            Ok(removed)
        });
        let removed = match result {
            Err(PieFail::Busy) => {
                core::hint::spin_loop();
                continue;
            }
            result => result??,
        };
        if let Some(p) = removed.snapshot().pole() {
            let _ = crate::work::mail::pole::shut(&p, token);
        }
        for (holder, _) in tasks {
            let _ = crate::work::room::messenger::signal(
                crate::work::room::messenger::WakeKey::Capabilities {
                    task: holder.ident.id,
                },
            );
        }
        return Ok(());
    }
    Err(PieFail::Busy)
}

pub(crate) fn same(task: &Arc<Task>, a: PieToken, b: PieToken) -> Result<bool, PieFail> {
    let pies = task.gate.pies.lock();
    let find = |token| {
        pies.iter()
            .find(|p| p.token() == token)
            .ok_or(PieFail::Denied)
    };
    let (a, b) = (find(a)?.snapshot(), find(b)?.snapshot());
    if !a.alive() || !b.alive() {
        return Err(PieFail::Dead);
    }
    Ok(a.same(&b))
}
