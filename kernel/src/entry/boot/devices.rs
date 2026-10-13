use crate::machine::board;
use crate::entry::boot::resources::Registry;
use crate::core::work::mail::{self, Mail};
use crate::core::work::pend::{self, Permission};
use env::{Mark, Name, Page, PieFail, TaskId};

pub(crate) fn register(registry: &mut Registry) -> Result<(), PieFail> {
    let dtb = board::info().dtb();
    let fdt = board::device_tree().ok_or(PieFail::Denied)?;
    for node in fdt.all_nodes() {
        if exempt(node.name) {
            continue;
        }
        let Some(reg) = node.reg() else {
            continue;
        };
        for r in reg {
            let base = r.starting_address.addr();
            let Some(size) = r.size else {
                continue;
            };
            if base == 0 || size == 0 {
                continue;
            }
            let Ok(meta) = mail::pole::region(base, size, TaskId::new(0)) else {
                continue;
            };
            let root = pend::Pie::try_new(
                Mail::Pole(meta),
                Mark::NONE,
                Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
                None,
            )?;
            registry
                .register(Name::Page(Page::Region(base as u64)), root)
                .map_err(|e| e.into_parts().0)?;
        }
    }
    let meta = mail::pole::region(dtb.base, dtb.size, TaskId::new(0))?;
    let root = pend::Pie::try_new(
        Mail::Pole(meta),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )?;
    registry
        .register(Name::Page(Page::Dtb), root)
        .map_err(|e| e.into_parts().0)?;
    if let Some(initrd) = board::info().initrd() {
        let meta = mail::pole::region(initrd.base, initrd.size, TaskId::new(0))?;
        let root = pend::Pie::try_new(
            Mail::Pole(meta),
            Mark::NONE,
            Permission::FETCH | Permission::VEST,
            None,
        )?;
        registry
            .register(Name::Page(Page::Initrd), root)
            .map_err(|e| e.into_parts().0)?;
    }
    Ok(())
}
fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}
