pub(crate) mod conductor;
pub mod messenger;
pub mod scheduler;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::work::unit::task::Task;

pub(crate) struct Chain {
    head: Option<Arc<Task>>,
    tail: Option<Arc<Task>>,
}

impl Chain {
    pub(crate) const fn new() -> Self {
        Self {
            head: None,
            tail: None,
        }
    }

    pub(crate) fn push(&mut self, task: Arc<Task>) {
        match self.tail.take() {
            None => self.head = Some(task.clone()),
            Some(last) => last.set_next(Some(task.clone())),
        }
        self.tail = Some(task);
    }

    pub(crate) fn pop(&mut self) -> Option<Arc<Task>> {
        let head = self.head.take()?;
        self.head = head.take_next();
        if self.head.is_none() {
            self.tail = None;
        }
        Some(head)
    }

    pub(crate) fn take(&mut self) -> Option<Arc<Task>> {
        self.tail = None;
        self.head.take()
    }

    pub(crate) fn remove_if(
        &mut self,
        hit: &mut dyn FnMut(&mut Arc<Task>) -> bool,
    ) -> Option<Arc<Task>> {
        let mut prev: Option<Arc<Task>> = None;
        let mut cur = self.head.clone();
        while let Some(mut node) = cur {
            if hit(&mut node) {
                let next = node.take_next();
                let was_tail = next.is_none();
                match &mut prev {
                    Some(p) => p.set_next(next),
                    None => self.head = next,
                }
                if was_tail {
                    self.tail = prev;
                }
                return Some(node);
            }
            prev = Some(node.clone());
            cur = node.next();
        }
        None
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn len(&self) -> usize {
        let mut n = 0usize;
        let mut cur = self.head.clone();
        while let Some(node) = cur {
            n += 1;
            cur = node.next();
        }
        n
    }
}

pub(crate) struct Counter(AtomicUsize);

impl Counter {
    pub(crate) const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub(crate) fn bump(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}
