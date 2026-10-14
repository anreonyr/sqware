use super::pie::Permission;
use crate::core::work::grant::narrows;
use crate::core::work::mail::pole::PoleOperation;
use env::PieFail;

pub(crate) fn reduce(
    task: &alloc::sync::Arc<crate::core::work::unit::task::Task>,
    token: env::PieToken,
    subset: Permission,
) -> Result<(), PieFail> {
    reduce_locked(task, token, subset)?;
    super::notify(task.ident.id, token);
    Ok(())
}

fn reduce_locked(
    task: &alloc::sync::Arc<crate::core::work::unit::task::Task>,
    token: env::PieToken,
    subset: Permission,
) -> Result<(), PieFail> {
    use crate::core::memory::nest::Access;
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
        let access = Access::READ
            | if subset.contains(Permission::STORE) {
                Access::WRITE
            } else {
                Access::empty()
            };
        let meta = &p;
        let _operation = PoleOperation::acquire(meta).ok_or(PieFail::Busy)?;
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
        crate::core::work::mail::pole::narrow(meta, token, access)?;
        let _pend = task.pend.lock();
        let _commit = crate::core::work::commit();
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
    let _commit = crate::core::work::commit();
    let mut pies = task.pend.pies.lock();
    let pie = pies
        .iter_mut()
        .find(|pie| pie.token() == token)
        .ok_or(PieFail::Denied)?;
    pie.narrow(subset)?;
    super::changed(task);
    Ok(())
}
