use ::resource::pile::{Pile, Sub};
use alloc::vec::Vec;
use env::{HoleDir, PieToken};
pub(crate) struct Waiting {
    pile: Pile,
    members: Vec<PieToken>,
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
        let _ = self.pile.detach(token, HoleDir::Pull);
    }
    pub(super) fn await_(&self, wait: env::Wait) -> Result<(), ()> {
        self.pile.await_(wait).map(|_| ()).map_err(|_| ())
    }
    pub fn apply(&mut self, tokens: &[PieToken], subs: &[Sub]) -> bool {
        let mut at = 0;
        while at < self.members.len() {
            if tokens.contains(&self.members[at]) {
                at += 1;
            } else {
                let token = self.members.swap_remove(at);
                let _ = self.pile.detach(token, HoleDir::Pull);
            }
        }
        for &token in tokens {
            if self.members.contains(&token) {
                continue;
            }
            if self.pile.attach(token, HoleDir::Pull).is_err() {
                return false;
            }
            self.members.push(token);
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
