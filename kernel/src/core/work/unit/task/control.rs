use alloc::sync::Arc;
use env::ViewControl;

use crate::core::memory::nest::NestError;
use crate::core::work::unit::space::Space;

use super::Task;

impl Task {
    pub(crate) fn track_control(&self, space: &Arc<Space>, id: usize) -> Result<(), NestError> {
        let mut controls = self.controls.lock();
        controls
            .try_reserve(1)
            .map_err(|_| NestError::OutOfMemory)?;
        controls.push((Arc::downgrade(space), id));
        Ok(())
    }

    pub(crate) fn release_control(&self, id: usize) {
        self.controls.lock().retain(|(_, held)| *held != id);
    }

    pub(crate) fn close_controls(&self) {
        let controls = core::mem::take(&mut *self.controls.lock());
        for (space, id) in controls {
            let Some(space) = space.upgrade() else {
                continue;
            };
            let Some(entry) = space.views.locate(id) else {
                continue;
            };
            if entry.controller != self.ident.id || entry.scope != ViewControl::Caller {
                continue;
            }
            if let Some(view) = entry.view() {
                let _ = space.nest.dispose(view.clone());
            }
            let _ = space.nest.retry_invalidation();
            space.reap_views();
        }
    }
}
