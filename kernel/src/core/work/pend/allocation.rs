use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU8, Ordering};

use crate::core::memory::manager::Map;
use crate::machine::sync::SpinLock;

const AVAILABLE: u8 = 0;
const CLAIMED: u8 = 1;
const CONSUMED: u8 = 2;

struct Creation {
    state: AtomicU8,
}

struct Node {
    map: Weak<Map>,
    creation: Arc<Creation>,
    next: Option<Box<Node>>,
}

pub(crate) struct CreationRegistry {
    nodes: SpinLock<Option<Box<Node>>>,
}

pub(crate) struct PreparedAllocation<'a> {
    registry: &'a CreationRegistry,
    node: Option<Box<Node>>,
}

pub(crate) struct CreationLease<'a> {
    registry: &'a CreationRegistry,
    creation: Arc<Creation>,
    committed: bool,
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
        let creation = Arc::try_new(Creation {
            state: AtomicU8::new(AVAILABLE),
        })
        .map_err(|_| env::PieFail::OoM)?;
        let node = Box::try_new(Node {
            map: Weak::new(),
            creation,
            next: None,
        })
        .map_err(|_| env::PieFail::OoM)?;
        Ok(PreparedAllocation {
            registry: self,
            node: Some(node),
        })
    }

    pub(crate) fn claim(&self, map: &Arc<Map>) -> Result<CreationLease<'_>, env::PieFail> {
        let nodes = self.nodes.lock();
        let node = chain(nodes.as_deref())
            .find(|node| {
                node.map
                    .upgrade()
                    .is_some_and(|held| Arc::ptr_eq(&held, map))
            })
            .ok_or(env::PieFail::Denied)?;
        match node.creation.state.compare_exchange(
            AVAILABLE,
            CLAIMED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(CreationLease {
                registry: self,
                creation: node.creation.clone(),
                committed: false,
            }),
            Err(CLAIMED) => Err(env::PieFail::Busy),
            Err(_) => Err(env::PieFail::Denied),
        }
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

impl PreparedAllocation<'_> {
    pub(crate) fn commit(mut self, map: &Arc<Map>) {
        let mut node = self.node.take().expect("prepared creation record");
        node.map = Arc::downgrade(map);
        let mut head = self.registry.nodes.lock();
        if chain(head.as_deref()).any(|node| {
            node.map
                .upgrade()
                .is_some_and(|held| Arc::ptr_eq(&held, map))
        }) {
            return;
        }
        node.next = head.take();
        *head = Some(node);
    }
}

impl CreationLease<'_> {
    pub(crate) fn commit(mut self) {
        self.creation.state.store(CONSUMED, Ordering::Release);
        self.registry.remove(&self.creation);
        self.committed = true;
    }
}

impl Drop for CreationLease<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.creation.state.compare_exchange(
                CLAIMED,
                AVAILABLE,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
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
}
