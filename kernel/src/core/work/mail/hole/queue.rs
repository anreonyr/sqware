use alloc::collections::VecDeque;

use env::{HoleLimits, MailCondition};

use super::hand::Hand;
use super::slot::Slot;

#[derive(Default)]
pub(super) struct Queue {
    pub(super) hands: VecDeque<Slot>,
    pub(super) taking: bool,
    pub(super) bytes: usize,
}

impl Queue {
    pub(super) fn ready(&self, dir: MailCondition, limits: &HoleLimits) -> bool {
        match dir {
            MailCondition::Pull => self.head().is_some() && !self.taking,
            MailCondition::Signal(_) => false,
            MailCondition::Push => {
                self.hands.len() < limits.max_messages && self.bytes < limits.max_bytes
            }
            MailCondition::Empty => self.hands.is_empty() && !self.taking,
        }
    }

    pub(super) fn head(&self) -> Option<&Hand> {
        self.hands.front().and_then(Slot::hand)
    }
}
