use env::{Mark, Permission, PieKind, PieToken, TaskId};
use resource::dock::Dock;
use resource::raw::pies;

pub fn claim(system: TaskId, slot: Mark) -> Result<PieToken, &'static str> {
    let manifest = pies()
        .find(|info| {
            info.alive
                && info.kind == PieKind::Pole
                && info.owner == system
                && info.vestor == system
                && info.mark == slot
                && info.permission == Permission::FETCH
        })
        .ok_or("driver: retention manifest")?;
    let dock = Dock::open(manifest.token).map_err(|_| "driver: retention manifest open")?;
    let view = dock.view();
    let valid = view.size() >= 1
        // SAFETY: The live read-only manifest View covers this byte.
        && unsafe { core::ptr::read_volatile(view.base() as *const u8) == 1 };
    dock.shut()
        .map_err(|_| "driver: retention manifest close")?;
    if !valid {
        return Err("driver: retention collection");
    }
    pies()
        .find(|info| {
            info.alive
                && info.kind == PieKind::Nole
                && info.vestor == system
                && info.mark == Mark::of("memory/retention")
                && info.permission == (Permission::FETCH | Permission::VEST)
        })
        .map(|info| info.token)
        .ok_or("driver: retention authority")
}
