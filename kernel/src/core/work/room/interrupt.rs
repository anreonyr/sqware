use alloc::sync::Arc;
use ::core::sync::atomic::{AtomicUsize, Ordering};
use crate::core::work::mail::{HoleMeta, nole::NoleMeta};
use crate::machine::sync::OnceLock;
use env::{PieFail, TaskId};

pub(crate) struct Interrupts {
    pub supervisor_external: Arc<NoleMeta>,
    pub page_fault: Arc<HoleMeta>,
}

static INTERRUPTS: OnceLock<Interrupts> = OnceLock::new();
static RING: AtomicUsize = AtomicUsize::new(0);
static BUSY: AtomicUsize = AtomicUsize::new(0);
static IDLE_RING: AtomicUsize = AtomicUsize::new(0);
static IDLE_BUSY: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn init() -> Result<(), PieFail> {
    if INTERRUPTS.get().is_some() {
        return Ok(());
    }
    let interrupts = Interrupts {
        supervisor_external: NoleMeta::try_new(TaskId::new(0)).map_err(|_| PieFail::OoM)?,
        page_fault: HoleMeta::try_new(TaskId::new(0)).map_err(|_| PieFail::OoM)?,
    };
    assert!(INTERRUPTS.set(interrupts).is_ok(), "trap resources already initialized");
    Ok(())
}

pub(crate) fn get() -> &'static Interrupts {
    INTERRUPTS.get().expect("trap resources not initialized")
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
