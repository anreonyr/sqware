use super::{cache_step::CacheStep, fence::Fence, queue::CommandQueue, ticket::Ticket};

/// Admission progress and completion for one device context's cache barrier.
pub struct Invalidation {
    requester: u16,
    pscid: u32,
    next: CacheStep,
    ticket: Ticket,
}

impl Invalidation {
    pub fn new(requester: u16, pscid: u32, ticket: Ticket) -> Option<Self> {
        if super::context::offset(requester).is_none() || pscid >= super::context::PSCID_LIMIT {
            return None;
        }
        Some(Self { requester, pscid, next: CacheStep::Directory, ticket })
    }

    pub fn step(&mut self, queues: &mut CommandQueue) -> Fence {
        if !queues.accepts(&self.ticket) { return Fence::Expired; }
        loop {
            match self.next {
                CacheStep::Directory => {
                    super::order::sync();
                    if !queues.invalidate_context(self.requester) { return Fence::Waiting; }
                    self.next = CacheStep::Translations;
                }
                CacheStep::Translations => {
                    if !queues.invalidate_translations(self.pscid) { return Fence::Waiting; }
                    self.next = CacheStep::Fence;
                }
                CacheStep::Fence => return queues.fence_step(&mut self.ticket),
            }
        }
    }
}
