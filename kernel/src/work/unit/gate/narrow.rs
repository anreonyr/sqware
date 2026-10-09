use super::pie::{AnyPie, Mail, Permission, Pie, PieType};
use env::PieFail;

fn set_perm<T: PieType>(pie: &mut Pie<T>, subset: Permission) -> Result<(), PieFail> {
    if !pie.meta.alive() {
        return Err(PieFail::Dead);
    }
    if subset.is_empty() || (subset & pie.permission) != subset {
        return Err(PieFail::Denied);
    }
    if pie.permission.contains(Permission::ONLY) && !subset.contains(Permission::ONLY) {
        return Err(PieFail::Denied);
    }
    pie.permission = subset;
    if let Some(permit) = &pie.permit {
        permit.narrow(subset);
    }
    Ok(())
}

pub(crate) fn narrow(src: &mut AnyPie, subset: Permission) -> Result<(), PieFail> {
    match src {
        AnyPie::Hole(p) => set_perm(p, subset),
        AnyPie::Pole(p) => set_perm(p, subset),
        AnyPie::Nole(p) => set_perm(p, subset),
        AnyPie::Tole(p) => set_perm(p, subset),
    }
}

/// Lower token-associated views and staged program ceilings before removing authority.
pub(crate) fn reduce(
    task: &alloc::sync::Arc<crate::work::unit::task::Task>,
    token: env::PieToken,
    subset: Permission,
) -> Result<(), PieFail> {
    reduce_locked(task, token, subset)?;
    // 表里那一枚**就地**缩了权：释放任务的 `gate`/`pies` 之后要求持有者复核一次。
    let _ =
        crate::work::room::messenger::signal(crate::work::room::messenger::WakeKey::Capabilities {
            task: task.ident.id,
        });
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
    if let AnyPie::Pole(p) = &pie {
        if !subset.contains(Permission::FETCH) {
            return Err(PieFail::Denied);
        }
        let flags = PteFlags::R
            | if subset.contains(Permission::STORE) {
                PteFlags::W
            } else {
                PteFlags::empty()
            };
        let meta = p.meta();
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
        let mut pies = task.pies.lock();
        let pie = pies
            .iter_mut()
            .find(|pie| pie.token() == token)
            .ok_or(PieFail::Denied)?;
        narrow(pie, subset)?;
        super::changed(task);
        return Ok(());
    }
    let _gate = task.gate.lock();
    let mut pies = task.pies.lock();
    let pie = pies
        .iter_mut()
        .find(|pie| pie.token() == token)
        .ok_or(PieFail::Denied)?;
    narrow(pie, subset)?;
    super::changed(task);
    Ok(())
}
