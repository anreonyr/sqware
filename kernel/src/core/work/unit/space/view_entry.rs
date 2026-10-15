use alloc::sync::{Arc, Weak};
use env::{PieToken, TaskId, ViewControl};

use crate::core::memory::nest::View;
use crate::core::work::pend::SourceAuthorization;

use super::Space;

#[derive(Clone)]
pub(crate) struct ViewEntry {
    pub(crate) space: Weak<Space>,
    pub(crate) view: Option<Arc<View>>,
    pub(crate) scope: ViewControl,
    pub(crate) controller: TaskId,
    pub(crate) source: Arc<SourceAuthorization>,
    pub(crate) branch: Option<PieToken>,
}

impl ViewEntry {
    pub(crate) fn view(&self) -> Option<&Arc<View>> {
        self.view.as_ref()
    }
}
