use alloc::boxed::Box;
use alloc::sync::Arc;

use crate::core::memory::manager::Map;

use super::{CreationRegistry, Node, chain};

pub(crate) struct PreparedAllocation<'a> {
    pub(super) registry: &'a CreationRegistry,
    pub(super) node: Box<Node>,
}

impl PreparedAllocation<'_> {
    pub(crate) fn commit(self, map: &Arc<Map>) {
        let mut node = self.node;
        node.map = Arc::downgrade(map);
        let mut head = self.registry.nodes.lock();
        if chain(head.as_deref()).any(|node| node.matches(map)) {
            return;
        }
        node.next = head.take();
        *head = Some(node);
    }
}
