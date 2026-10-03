use core::ptr::NonNull;

use fack::prelude::Error;

pub mod bitmap;
pub mod block;
pub mod bump;
pub mod frame;
pub mod hybrid;
pub mod portal;
pub mod spare;
pub mod statistics;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[allow(unused)]
pub enum InitError {
    #[error("no free memory region configured")]
    NoFreeMemory,
    #[error("memory allocation failed while initializing allocator")]
    OutOfMemory,
    #[error("no free physical frames available")]
    NoFreeFrames,
    #[error("no harts reported")]
    NoHarts,
    #[error("allocator already initialized")]
    AlreadyInitialized,
}

pub type InitResult<T> = erra::Result<T, InitError>;

struct Link {
    prev: Option<NonNull<Link>>,
    next: Option<NonNull<Link>>,
}

impl Link {
    fn new(prev: Option<NonNull<Link>>, next: Option<NonNull<Link>>) -> Self {
        Self { prev, next }
    }
}

pub fn init() -> InitResult<()> {
    bump::init()?;
    portal::switch(portal::Backend::Bump);

    statistics::init().expect("statistics init: already initialized");

    hybrid::init()?;
    portal::switch(portal::Backend::Hybrid);
    spare::init()?;

    statistics::record_spare_total(spare::spare().total_bytes());

    Ok(())
}

/// Debug guard for publication sections whose capacity has already been reserved.
#[cfg(debug_assertions)]
static FORBIDDEN: [core::sync::atomic::AtomicBool; crate::layout::MAX_HART_SLOTS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; crate::layout::MAX_HART_SLOTS];

#[cfg(debug_assertions)]
pub(crate) struct NoAllocation(crate::hart::HartId);

#[cfg(debug_assertions)]
impl NoAllocation {
    pub(crate) fn enter() -> Self {
        let hart = crate::hart::hart_id();
        assert!(!FORBIDDEN[hart.get()].swap(true, core::sync::atomic::Ordering::Relaxed));
        Self(hart)
    }
}

#[cfg(debug_assertions)]
impl Drop for NoAllocation {
    fn drop(&mut self) { FORBIDDEN[self.0.get()].store(false, core::sync::atomic::Ordering::Relaxed); }
}

#[inline]
pub(crate) fn assert_allocation_allowed() {
    #[cfg(debug_assertions)]
    assert!(!FORBIDDEN[crate::hart::hart_id().get()].load(core::sync::atomic::Ordering::Relaxed),
        "allocation or shootdown in publication commit");
}
