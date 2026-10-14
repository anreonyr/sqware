use alloc::vec::Vec;
use env::{Entry, Name, PieFail};

use crate::core::work::pend::{self, Pie};
use crate::core::work::unit::task::Task;

use super::{Failure, GrantError, valid};

pub(crate) struct Frozen {
    pub(super) entries: Vec<(Name, Pie)>,
}

impl Frozen {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn grant(self, target: &Task) -> Result<Vec<Entry>, GrantError> {
        let closed = target.pend.lock();
        let commit = crate::core::work::commit();
        if !target.held() || self.entries.iter().any(|(n, p)| !valid(*n, p)) {
            return Err(Failure {
                reason: PieFail::Denied,
                value: self,
            });
        }
        let mut ledger = Vec::new();
        if ledger.try_reserve_exact(self.entries.len()).is_err() {
            return Err(Failure {
                reason: PieFail::OoM,
                value: self,
            });
        }
        let mut pies = target.pend.pies.lock();
        if pies.try_reserve(self.entries.len()).is_err() {
            return Err(Failure {
                reason: PieFail::OoM,
                value: self,
            });
        }
        for (name, root) in self.entries {
            ledger.push(Entry::new(name, root.mail().kind(), root.token()));
            pies.push(root);
        }
        drop(pies);
        pend::changed(target);
        drop(commit);
        drop(closed);
        Ok(ledger)
    }
}
