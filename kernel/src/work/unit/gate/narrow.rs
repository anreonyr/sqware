use super::pie::{AnyPie, Permission};
use env::PieFail;

pub(crate) fn narrow(src: &mut AnyPie, subset: Permission) -> Result<(), PieFail> {
    src.narrow(subset)
}

/// Lower token-associated views and staged program ceilings before removing authority.
pub(crate) fn reduce(
    task: &alloc::sync::Arc<crate::work::unit::task::Task>,
    token: env::PieToken,
    subset: Permission,
) -> Result<(), PieFail> {
    reduce_locked(task, token, subset)?;
    super::notify(task.ident.id, token);
    Ok(())
}

fn reduce_locked(
    task: &alloc::sync::Arc<crate::work::unit::task::Task>,
    token: env::PieToken,
    subset: Permission,
) -> Result<(), PieFail> {
    use crate::memory::manager::entry::PteFlags;
    let pie = super::locate(task, token).ok_or(PieFail::Denied)?;
    if !pie.alive() {
        return Err(PieFail::Dead);
    }
    if !pie.covers(subset)
        || pie.permission().contains(Permission::ONLY) && !subset.contains(Permission::ONLY)
    {
        return Err(PieFail::Denied);
    }
    if let Some(p) = pie.pole() {
        if !subset.contains(Permission::FETCH) {
            return Err(PieFail::Denied);
        }
        let flags = PteFlags::R
            | if subset.contains(Permission::STORE) {
                PteFlags::W
            } else {
                PteFlags::empty()
            };
        let meta = &p;
        let _operation = meta.backing().operation().ok_or(PieFail::Busy)?;
        let reserved = meta.backing().reserved();
        let target = if reserved == 0 {
            None
        } else {
            Some(
                task.heir(env::TeamId::new(reserved))
                    .ok_or(PieFail::Denied)?,
            )
        };
        let _construction = match &target {
            Some(target) => Some(target.operation().ok_or(PieFail::Busy)?),
            None => None,
        };
        if let Some(target) = &target {
            target
                .space
                .with_shootdown(|inner| inner.narrow_token(token, flags))
                .expect("staged narrow: shootdown failed")
                .map_err(|_| PieFail::Denied)?;
        }
        crate::work::mail::pole::narrow(meta, token, flags)?;
        let _gate = task.gate.lock();
    let _commit = super::super::commit();
        let mut pies = task.gate.pies.lock();
        let pie = pies
            .iter_mut()
            .find(|pie| pie.token() == token)
            .ok_or(PieFail::Denied)?;
        narrow(pie, subset)?;
        super::changed(task);
        return Ok(());
    }
    let _gate = task.gate.lock();
    let _commit = super::super::commit();
    let mut pies = task.gate.pies.lock();
    let pie = pies
        .iter_mut()
        .find(|pie| pie.token() == token)
        .ok_or(PieFail::Denied)?;
    narrow(pie, subset)?;
    super::changed(task);
    Ok(())
}
