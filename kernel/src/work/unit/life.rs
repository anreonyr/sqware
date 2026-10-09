use alloc::sync::{Arc, Weak};

use env::TaskId;

#[derive(Debug)]
pub(crate) struct Life;

impl Life {
    #[cfg(debug_assertions)]
    pub(crate) fn new() -> Arc<Life> {
        Arc::new(Life)
    }

    #[allow(dead_code)]
    pub(crate) fn try_new() -> Result<Arc<Life>, crate::memory::manager::MapError> {
        Arc::try_new_in(Life, alloc::alloc::Global)
            .map_err(|_| crate::memory::manager::MapError::OutOfMemory)
    }

    pub(crate) fn live(w: &Weak<Life>) -> bool {
        w.strong_count() > 0
    }

    pub(crate) fn dead(w: &Weak<Life>) -> bool {
        !Self::live(w)
    }
}

pub(crate) struct TaskLife {
    pub(crate) id: TaskId,
    pub(crate) life: Weak<Life>,
}
