//! 任务链：`Arc<Task>` 沿 `Task::next` 串成的 FIFO。
//!
//! 饥饿队列、等待站点、收尸壳都用它装任务，故归 `room` 这一层。

use alloc::sync::Arc;

use crate::core::work::unit::task::Task;

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
