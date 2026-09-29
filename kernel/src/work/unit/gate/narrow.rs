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
