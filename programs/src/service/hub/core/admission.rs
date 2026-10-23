//! One execution instance admitted by Hub for DMA claims.
//!
//! The record only holds Hub's admission facts: identity, claim watermark, admission
//! deadline and whether the admission is closed. Team identity, running state and death
//! stay with Control.

use env::TaskId;

pub struct Admission {
    pub task: TaskId,
    deadline: u64,
    claim_high: u64,
    closed: bool,
}

impl Admission {
    pub fn new(task: TaskId, deadline: u64) -> Self {
        Self {
            task,
            deadline,
            claim_high: 0,
            closed: false,
        }
    }

    pub fn deadline(&self) -> u64 {
        self.deadline
    }

    pub fn claim_high(&self) -> u64 {
        self.claim_high
    }

    pub fn advance_claim(&mut self, id: u64) {
        self.claim_high = id;
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    pub fn claimable(&self) -> bool {
        !self.closed
    }

    pub fn expired(&self, now: u64) -> bool {
        !self.closed && self.claim_high == 0 && now >= self.deadline
    }
}
