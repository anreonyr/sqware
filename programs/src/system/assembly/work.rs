use alloc::collections::VecDeque;
use core::ops::{Deref, DerefMut};

pub(crate) struct WorkQueue<T> {
    entries: VecDeque<T>,
    next: u64,
}

pub(crate) trait JobIdentity {
    fn completed_id(&self) -> Option<super::JobId>;
}
pub(crate) struct Completed<T> {
    pub(crate) id: super::JobId,
    pub(crate) result: T,
}
impl<T> JobIdentity for Completed<T> {
    fn completed_id(&self) -> Option<super::JobId> {
        Some(self.id)
    }
}

impl<T> Default for WorkQueue<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next: 1,
        }
    }
}

impl<T> WorkQueue<T> {
    pub(crate) fn reserve_job(&mut self, limit: usize) -> Option<super::JobId> {
        if self.entries.len() >= limit || self.entries.try_reserve(1).is_err() {
            return None;
        }
        let id = super::JobId::new(self.next)?;
        self.next = self.next.checked_add(1)?;
        Some(id)
    }
}

impl<T: JobIdentity> WorkQueue<T> {
    pub(crate) fn take(&mut self, id: super::JobId) -> Option<T> {
        let at = self
            .entries
            .iter()
            .position(|item| item.completed_id() == Some(id))?;
        self.entries.remove(at)
    }
}

impl<T> WorkQueue<T> {
    pub(crate) fn pop_matching(
        &mut self,
        rounds: usize,
        mut matches: impl FnMut(&T) -> bool,
    ) -> Option<T> {
        let rounds = rounds.min(self.entries.len());
        for _ in 0..rounds {
            let item = self.entries.pop_front()?;
            if matches(&item) {
                return Some(item);
            }
            self.entries.push_back(item);
        }
        None
    }
}

impl<T> Deref for WorkQueue<T> {
    type Target = VecDeque<T>;
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<T> DerefMut for WorkQueue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}
