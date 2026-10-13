use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU8, Ordering};

use crate::lock::SpinLock;
use crate::memory::manager::Map;

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

/// Records which eagerly allocated Map may become a root Pole.
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

impl CreationRegistry {
    pub(crate) fn new() -> Self {
        Self {
            nodes: SpinLock::new(None),
        }
    }

    /// Reserve every registry allocation before publishing the page-table map.
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
        let mut cursor = nodes.as_deref();
        while let Some(node) = cursor {
            if node
                .map
                .upgrade()
                .is_some_and(|registered| Arc::ptr_eq(&registered, map))
            {
                return match node.creation.state.compare_exchange(
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
                };
            }
            cursor = node.next.as_deref();
        }
        Err(env::PieFail::Denied)
    }

    pub(crate) fn locate(
        &self,
        start: crate::memory::manager::addr::VirtAddr,
        size: usize,
    ) -> Option<Arc<Map>> {
        let nodes = self.nodes.lock();
        let mut cursor = nodes.as_deref();
        while let Some(node) = cursor {
            if let Some(map) = node.map.upgrade() {
                if map.initial_range() == (start, size) {
                    return Some(map);
                }
            }
            cursor = node.next.as_deref();
        }
        None
    }

    pub(crate) fn forget(&self, map: &Arc<Map>) {
        let creation = {
            let nodes = self.nodes.lock();
            let mut cursor = nodes.as_deref();
            let mut found = None;
            while let Some(node) = cursor {
                if node
                    .map
                    .upgrade()
                    .is_some_and(|registered| Arc::ptr_eq(&registered, map))
                {
                    found = Some(node.creation.clone());
                    break;
                }
                cursor = node.next.as_deref();
            }
            found
        };
        if let Some(creation) = creation {
            creation.state.store(CONSUMED, Ordering::Release);
            self.remove(&creation);
        }
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
    /// Install the preallocated record after the Map has been published.
    pub(crate) fn commit(mut self, map: &Arc<Map>) {
        let mut node = self.node.take().expect("prepared creation record");
        node.map = Arc::downgrade(map);
        let mut head = self.registry.nodes.lock();
        let mut cursor = head.as_deref();
        while let Some(existing) = cursor {
            if existing
                .map
                .upgrade()
                .is_some_and(|registered| Arc::ptr_eq(&registered, map))
            {
                return;
            }
            cursor = existing.next.as_deref();
        }
        node.next = head.take();
        *head = Some(node);
    }
}

impl CreationLease<'_> {
    /// Consume the one-time right once Pole construction commits.
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
    use crate::memory::manager::Map;

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
