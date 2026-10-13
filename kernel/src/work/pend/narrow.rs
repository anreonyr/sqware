use super::pie::Permission;
use crate::work::grant::narrows;
use env::PieFail;

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
    if !narrows(pie.permission(), subset) {
        return Err(PieFail::Denied);
    }
    if let Some(p) = pie.mail().pole() {
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
        let _operation = meta.operation().ok_or(PieFail::Busy)?;
        let reserved = meta.reserved();
        let target = if reserved == 0 {
            None
        } else {
            Some(
                task.clan(env::TeamId::new(reserved))
                    .ok_or(PieFail::Denied)?,
            )
        };
        let _construction = match &target {
            Some(target) => Some(target.operation().ok_or(PieFail::Busy)?),
            None => None,
        };
        crate::work::mail::pole::narrow(meta, token, flags)?;
        let _pend = task.pend.lock();
        let _commit = crate::work::unit::commit();
        let mut pies = task.pend.pies.lock();
        let pie = pies
            .iter_mut()
            .find(|pie| pie.token() == token)
            .ok_or(PieFail::Denied)?;
        pie.narrow(subset)?;
        super::changed(task);
        return Ok(());
    }
    let _pend = task.pend.lock();
    let _commit = crate::work::unit::commit();
    let mut pies = task.pend.pies.lock();
    let pie = pies
        .iter_mut()
        .find(|pie| pie.token() == token)
        .ok_or(PieFail::Denied)?;
    pie.narrow(subset)?;
    super::changed(task);
    Ok(())
}
