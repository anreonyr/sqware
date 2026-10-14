use alloc::sync::{Arc, Weak};

pub(crate) use super::task::life::TaskLife;
pub(crate) use super::team::life::TeamLife;

#[derive(Debug)]
pub(crate) struct Life;

impl Life {
    #[cfg(debug_assertions)]
    pub(crate) fn new() -> Arc<Life> {
        Arc::new(Life)
    }

    pub(crate) fn try_new() -> Result<Arc<Life>, crate::core::memory::manager::MapError> {
        Arc::try_new_in(Life, alloc::alloc::Global)
            .map_err(|_| crate::core::memory::manager::MapError::OutOfMemory)
    }

    pub(crate) fn live(w: &Weak<Life>) -> bool {
        w.strong_count() > 0
    }

    pub(crate) fn dead(w: &Weak<Life>) -> bool {
        !Self::live(w)
    }
}
