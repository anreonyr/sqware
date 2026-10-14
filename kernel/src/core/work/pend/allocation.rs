use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};

use crate::core::memory::manager::Map;
use crate::machine::sync::SpinLock;

mod creation;
mod lease;
mod node;
mod prepared;

use creation::Creation;
pub(crate) use lease::CreationLease;
use node::Node;
pub(crate) use prepared::PreparedAllocation;

pub(crate) struct CreationRegistry {
    nodes: SpinLock<Option<Box<Node>>>,
}

fn chain(head: Option<&Node>) -> impl Iterator<Item = &Node> {
    core::iter::successors(head, |node| node.next.as_deref())
}

impl CreationRegistry {
    pub(crate) fn new() -> Self {
        Self {
            nodes: SpinLock::new(None),
        }
    }

    pub(crate) fn prepare(&self) -> Result<PreparedAllocation<'_>, env::PieFail> {
        let creation = Arc::try_new(Creation::new())
        .map_err(|_| env::PieFail::OoM)?;
        let node = Box::try_new(Node {
            map: Weak::new(),
            creation,
            next: None,
        })
        .map_err(|_| env::PieFail::OoM)?;
        Ok(PreparedAllocation {
            registry: self,
            node,
        })
    }

    pub(crate) fn claim(&self, map: &Arc<Map>) -> Result<CreationLease<'_>, env::PieFail> {
        let nodes = self.nodes.lock();
        let node = chain(nodes.as_deref())
            .find(|node| node.matches(map))
            .ok_or(env::PieFail::Denied)?;
        node.creation.claim()?;
        Ok(CreationLease {
            registry: self,
            creation: node.creation.clone(),
        })
    }

    pub(crate) fn locate(
        &self,
        start: crate::core::memory::manager::addr::VirtAddr,
        size: usize,
    ) -> Option<Arc<Map>> {
        let nodes = self.nodes.lock();
        chain(nodes.as_deref()).find_map(|node| {
            let map = node.map.upgrade()?;
            (map.initial_range() == (start, size)).then_some(map)
        })
    }

    fn remove(&self, creation: &Arc<Creation>) {
        let mut nodes = self.nodes.lock();
        let mut link = &mut *nodes;
        loop {
            let found = link
                .as_ref()
                .is_some_and(|node| Arc::ptr_eq(&node.creation, creation));
            if found {
                let mut removed = link.take().expect("matching creation node");
                *link = removed.next.take();
                return;
            }
            let Some(node) = link.as_mut() else { return };
            link = &mut node.next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::manager::Map;

    #[test]
    fn creation_is_single_use_and_bound_to_one_map() {
        let registry = CreationRegistry::new();
        let root = Arc::new(Map);
        let derived = Arc::new(Map);
        registry.prepare().unwrap().commit(&root);
        registry.prepare().unwrap().commit(&root);

        assert!(matches!(
            registry.claim(&derived),
            Err(env::PieFail::Denied)
        ));
        let lease = registry.claim(&root).unwrap();
        assert!(matches!(registry.claim(&root), Err(env::PieFail::Busy)));
        lease.commit();
        assert!(matches!(registry.claim(&root), Err(env::PieFail::Denied)));
    }

    #[test]
    fn failed_claim_rolls_the_creation_right_back() {
        let registry = CreationRegistry::new();
        let root = Arc::new(Map);
        registry.prepare().unwrap().commit(&root);
        {
            let _lease = registry.claim(&root).unwrap();
        }
        registry
            .claim(&root)
            .expect("dropped lease restores availability")
            .commit();
        assert!(matches!(registry.claim(&root), Err(env::PieFail::Denied)));
    }

    #[test]
    fn committed_lease_cannot_restore_consumed_creation() {
        let registry = CreationRegistry::new();
        let root = Arc::new(Map);
        registry.prepare().unwrap().commit(&root);
        let lease = registry.claim(&root).unwrap();
        let creation = lease.creation.clone();
        lease.commit();
        assert!(matches!(creation.claim(), Err(env::PieFail::Denied)));
        assert!(matches!(registry.claim(&root), Err(env::PieFail::Denied)));
    }
}
