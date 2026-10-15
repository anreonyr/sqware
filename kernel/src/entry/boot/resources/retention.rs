use env::{Call, Entry, Mark, Name, Permission, PieFail, PieKind};

use crate::core::work::mail::{Mail, nole::NoleMeta};
use crate::core::work::pend::{self, Pie};
use crate::core::work::unit::task::Task;

/// 这一手往引导账里多写的格数（`ledger::size` 要按它算）。
pub(crate) const ENTRIES: usize = 1;

/// 把内核铸的独立保留授权交到**这一个**任务手里，并把它的账目行交回调用方。
///
/// 交付是**推**的：对象在这里铸，收件人就是调用方点的那个任务；没有请求面，
/// 于是没有“第一个来问的就给它”这条路。调用点只有引导装配那一处。
pub(crate) fn grant(target: &Task) -> Result<Entry, PieFail> {
    let meta = NoleMeta::mint_retention(target.ident.id).map_err(|_| PieFail::OoM)?;
    let pie = Pie::try_new(
        Mail::Nole(meta),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )?;
    let token = pie.token();
    pend::insert(target, pie)?;
    Ok(Entry::new(Name::Call(Call::Retain), PieKind::Nole, token))
}
