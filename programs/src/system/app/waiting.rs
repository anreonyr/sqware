use ::resource::pile::{Pile, Sub};
use alloc::vec::Vec;
use env::{MailCondition, PieToken};
pub(crate) struct Waiting {
    pile: Pile,
    members: Vec<(PieToken, MailCondition)>,
    subs: Vec<Sub>,
}
impl Waiting {
    pub(crate) fn new() -> Result<Self, ()> {
        Ok(Self {
            pile: Pile::unseal(false).map_err(|_| ())?,
            members: Vec::new(),
            subs: Vec::new(),
        })
    }
    pub(crate) fn detach(&self, token: PieToken) {
        let _ = self.pile.detach(token, MailCondition::Pull);
    }
    pub(super) fn await_(&self, wait: env::Wait) -> Result<(), ()> {
        self.pile.await_(wait).map(|_| ()).map_err(|_| ())
    }
    pub fn apply(&mut self, interests: (&[PieToken], &[PieToken]), subs: &[Sub]) -> bool {
        let (reads, writes) = interests;
        if self
            .members
            .try_reserve(reads.len() + writes.len())
            .is_err()
            || self.subs.try_reserve(subs.len()).is_err()
        {
            return false;
        }
        let mut at = 0;
        while at < self.members.len() {
            let (token, direction) = self.members[at];
            let wanted = match direction {
                MailCondition::Pull => reads,
                MailCondition::Push | MailCondition::Signal(_) => return false,
                MailCondition::Empty => writes,
            };
            if wanted.contains(&token) {
                at += 1;
            } else {
                self.members.swap_remove(at);
                let _ = self.pile.detach(token, direction);
            }
        }
        for (tokens, direction) in [(reads, MailCondition::Pull), (writes, MailCondition::Empty)] {
            for &token in tokens {
                if self.members.contains(&(token, direction)) {
                    continue;
                }
                if self.pile.attach(token, direction).is_err() {
                    return false;
                }
                self.members.push((token, direction));
            }
        }
        let mut at = 0;
        while at < self.subs.len() {
            if subs.contains(&self.subs[at]) {
                at += 1;
            } else {
                let sub = self.subs.swap_remove(at);
                let _ = self.pile.unsubscribe(sub);
            }
        }
        for &sub in subs {
            if self.subs.contains(&sub) {
                continue;
            }
            if self.pile.subscribe(sub).is_err() {
                return false;
            }
            self.subs.push(sub);
        }
        true
    }
}
