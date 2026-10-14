use alloc::boxed::Box;
use alloc::sync::Arc;

use crate::core::memory::nest::View;

use super::{CreationRegistry, Node, chain};

pub(crate) struct PreparedAllocation<'a> {
    pub(super) registry: &'a CreationRegistry,
    pub(super) node: Box<Node>,
}

impl PreparedAllocation<'_> {
    pub(crate) fn commit(self, view: &Arc<View>) {
        let mut node = self.node;
        node.view = Arc::downgrade(view);
        let mut head = self.registry.nodes.lock();
        if chain(head.as_deref()).any(|node| node.matches(view)) {
            return;
        }
        node.next = head.take();
        *head = Some(node);
    }
}
