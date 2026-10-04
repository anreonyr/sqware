use crate::platform::machine;
use crate::resource::Registry;
use crate::work::mail;
use crate::work::unit::gate::{self, AnyPie, Permission};
use env::{Mark, Name, Page, PieFail, TaskId};

pub(crate) fn register(registry: &mut Registry) -> Result<(), PieFail> {
    let dtb = machine::info().dtb();
    // SAFETY: boot retains the DTB physical region for the machine's lifetime.
    let fdt = unsafe { fdt::Fdt::from_ptr(dtb.base as *const u8) }.map_err(|_| PieFail::Denied)?;
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
            let root = gate::try_new_pie(
                meta,
                Mark::NONE,
                Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
                None,
            )?;
            registry
                .register(Name::Page(Page::Region(base as u64)), AnyPie::Pole(root))
                .map_err(|e| e.into_parts().0)?;
        }
    }
    let meta = mail::pole::region(dtb.base, dtb.size, TaskId::new(0))?;
    let root = gate::try_new_pie(meta, Mark::NONE, Permission::FETCH | Permission::VEST, None)?;
    registry
        .register(Name::Page(Page::Dtb), AnyPie::Pole(root))
        .map_err(|e| e.into_parts().0)?;
    if let Some(initrd) = machine::info().initrd() {
        let meta = mail::pole::region(initrd.base, initrd.size, TaskId::new(0))?;
        let root = gate::try_new_pie(meta, Mark::NONE, Permission::FETCH | Permission::VEST, None)?;
        registry
            .register(Name::Page(Page::Initrd), AnyPie::Pole(root))
            .map_err(|e| e.into_parts().0)?;
    }
    Ok(())
}
fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}
