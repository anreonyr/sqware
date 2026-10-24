use env::{Call, Entry, Mark, Name, Permission, PieFail, PieKind};

use crate::core::work::mail::{Mail, nole::NoleMeta};
use crate::core::work::pend::{self, Pie};
use crate::core::work::unit::task::Task;

pub(crate) const ENTRIES: usize = 1;

pub(crate) fn grant(target: &Task) -> Result<Entry, PieFail> {
    let meta = NoleMeta::mint_guard(target.ident.id).map_err(|_| PieFail::OoM)?;
    let pie = Pie::try_new(Mail::Nole(meta), Mark::NONE, Permission::FETCH, None)?;
    let token = pie.token();
    pend::insert(target, pie)?;
    Ok(Entry::new(Name::Call(Call::Guard), PieKind::Nole, token))
}
