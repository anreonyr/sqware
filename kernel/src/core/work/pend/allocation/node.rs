use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};

use crate::core::memory::nest::View;

use super::Creation;

pub(super) struct Node {
    pub(super) view: Weak<View>,
    pub(super) creation: Arc<Creation>,
    pub(super) next: Option<Box<Node>>,
}

impl Node {
    pub(super) fn matches(&self, view: &Arc<View>) -> bool {
        self.view
            .upgrade()
            .is_some_and(|held| Arc::ptr_eq(&held, view))
    }
}
