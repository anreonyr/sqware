use alloc::sync::Weak;

use env::{Mark, PieFail, PieToken};

use super::pie::{AnyPie, Heir, Need, Permission, new_pie};
use crate::work::room::messenger::{self, WakeKey};
use crate::work::unit::task::Task;

pub(crate) fn accord(
    caller: &Task,
    src: PieToken,
    dst: &Weak<Task>,
    subset: Permission,
    mark: Mark,
) -> Result<usize, PieFail> {
    let target = dst.upgrade().ok_or(PieFail::Denied)?;
    let granted = {
        let mut pies = caller.pies.lock();
        let pie = pies
            .iter_mut()
            .find(|p| p.token() == src)
            .ok_or(PieFail::Denied)?;
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if !pie.allows(Need::Grant) {
            return Err(PieFail::Denied);
        }
        if !pie.covers(subset) {
            return Err(PieFail::Denied);
        }
        if !super::pie::form_ok(pie.permission(), subset) {
            return Err(PieFail::Denied);
        }
        if pie.heir().is_some() {
            return Err(PieFail::HandedOver);
        }
        let badge = if mark == Mark::NONE { pie.mark() } else { mark };
        let granted = match &*pie {
            AnyPie::Hole(p) => AnyPie::Hole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Pole(p) => AnyPie::Pole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Nole(p) => AnyPie::Nole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Tole(p) => AnyPie::Tole(new_pie(p.meta().clone(), badge, subset, Some(src))),
        };
        if pie.permission().contains(Permission::ONLY) {
            let h = Heir {
                task: target.ident.id,
                token: granted.token(),
            };
            match pie {
                AnyPie::Hole(p) => p.heir = Some(h),
                AnyPie::Pole(p) => p.heir = Some(h),
                AnyPie::Nole(p) => p.heir = Some(h),
                AnyPie::Tole(p) => p.heir = Some(h),
            }
        }
        granted
    };
    let token = granted.token();
    let mut kids = target.pies.lock();
    if kids.try_reserve(1).is_err() {
        drop(kids);
        drop(granted);
        clear_heir(caller, src);
        return Err(PieFail::OoM);
    }
    kids.push(granted);
    drop(kids);
    let _ = messenger::wake(
        WakeKey::Pies {
            task: target.ident.id,
        },
        &target.life(),
    );
    Ok(token.get())
}

pub(crate) fn clear_heir(task: &Task, token: PieToken) -> bool {
    let mut pies = task.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    match pie {
        AnyPie::Hole(p) => p.heir.take().is_some(),
        AnyPie::Pole(p) => p.heir.take().is_some(),
        AnyPie::Nole(p) => p.heir.take().is_some(),
        AnyPie::Tole(p) => p.heir.take().is_some(),
    }
}
