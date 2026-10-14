use alloc::sync::Arc;

use super::{Creation, CreationRegistry};

pub(crate) struct CreationLease<'a> {
    pub(super) registry: &'a CreationRegistry,
    pub(super) creation: Arc<Creation>,
}

impl CreationLease<'_> {
    pub(crate) fn commit(self) {
        self.creation.consume();
        self.registry.remove(&self.creation);
    }
}

impl Drop for CreationLease<'_> {
    fn drop(&mut self) {
        self.creation.release();
    }
}
