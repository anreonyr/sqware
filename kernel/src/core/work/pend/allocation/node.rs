use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};

use crate::core::memory::manager::Map;

use super::Creation;

pub(super) struct Node {
    pub(super) map: Weak<Map>,
    pub(super) creation: Arc<Creation>,
    pub(super) next: Option<Box<Node>>,
}

impl Node {
    pub(super) fn matches(&self, map: &Arc<Map>) -> bool {
        self.map.upgrade().is_some_and(|held| Arc::ptr_eq(&held, map))
    }
}
