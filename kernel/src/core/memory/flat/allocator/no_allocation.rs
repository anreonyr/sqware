#[cfg(debug_assertions)]
static FORBIDDEN: [core::sync::atomic::AtomicBool; crate::machine::layout::MAX_HART_SLOTS] =
    [const { core::sync::atomic::AtomicBool::new(false) }; crate::machine::layout::MAX_HART_SLOTS];

#[cfg(debug_assertions)]
pub(crate) struct NoAllocation(crate::machine::hart::HartId);

#[cfg(debug_assertions)]
impl NoAllocation {
    pub(crate) fn enter() -> Self {
        let hart = crate::machine::hart::hart_id();
        assert!(!FORBIDDEN[hart.get()].swap(true, core::sync::atomic::Ordering::Relaxed));
        Self(hart)
    }
}

#[cfg(debug_assertions)]
impl Drop for NoAllocation {
    fn drop(&mut self) {
        FORBIDDEN[self.0.get()].store(false, core::sync::atomic::Ordering::Relaxed);
    }
}

#[inline]
pub(crate) fn assert_allocation_allowed() {
    #[cfg(debug_assertions)]
    assert!(
        !FORBIDDEN[crate::machine::hart::hart_id().get()].load(core::sync::atomic::Ordering::Relaxed),
        "allocation or shootdown in publication commit"
    );
}
