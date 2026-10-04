use crate::lock::OnceLock;
use crate::resource::Registry;
use crate::work::mail::nole::NoleMeta;
use crate::work::mail::{HoleMeta, hole, nole};
use crate::work::unit::gate::{self, AnyPie};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use env::{Mark, Name, Permission, PieFail, TaskId, Trap};

pub(crate) struct Resources {
    pub supervisor_external: Arc<NoleMeta>,
    pub page_fault: Arc<HoleMeta>,
}
static RESOURCES: OnceLock<Resources> = OnceLock::new();
static RING: AtomicUsize = AtomicUsize::new(0);
static BUSY: AtomicUsize = AtomicUsize::new(0);
static IDLE_RING: AtomicUsize = AtomicUsize::new(0);
static IDLE_BUSY: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn register(registry: &mut Registry) -> Result<(), PieFail> {
    let resources = get();
    let permission = Permission::FETCH | Permission::VEST;
    let external = gate::try_new_pie(
        resources.supervisor_external.clone(),
        Mark::NONE,
        permission,
        None,
    )?;
    registry
        .register(Name::Trap(Trap::SupervisorExternal), AnyPie::Nole(external))
        .map_err(|e| e.into_parts().0)?;
    let fault = gate::try_new_pie(resources.page_fault.clone(), Mark::NONE, permission, None)?;
    registry
        .register(Name::Trap(Trap::PageFault), AnyPie::Hole(fault))
        .map_err(|e| e.into_parts().0)?;
    Ok(())
}
pub(crate) fn init() {
    RESOURCES.get_or_init(|| Resources {
        supervisor_external: nole::NoleMeta::new(TaskId::new(0)),
        page_fault: hole::meta(TaskId::new(0)),
    });
}
pub(crate) fn get() -> &'static Resources {
    RESOURCES.get().expect("trap resources not initialized")
}
pub(crate) fn note_external(busy: bool, idle: bool) {
    RING.fetch_add(1, Ordering::Relaxed);
    if busy {
        BUSY.fetch_add(1, Ordering::Relaxed);
    }
    if idle {
        IDLE_RING.fetch_add(1, Ordering::Relaxed);
        if busy {
            IDLE_BUSY.fetch_add(1, Ordering::Relaxed);
        }
    }
}
pub(crate) fn stats() -> (usize, usize, usize, usize) {
    (
        RING.load(Ordering::Relaxed),
        BUSY.load(Ordering::Relaxed),
        IDLE_RING.load(Ordering::Relaxed),
        IDLE_BUSY.load(Ordering::Relaxed),
    )
}
