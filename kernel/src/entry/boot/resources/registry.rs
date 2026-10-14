use alloc::vec::Vec;
use env::{Name, PieFail};

use crate::core::work::pend::Pie;

use super::{Failure, FreezeError, Frozen, RegisterError, valid};

#[derive(Default)]
pub(crate) struct Registry {
    entries: Vec<(Name, Pie)>,
}

impl Registry {
    pub(crate) fn register(&mut self, name: Name, root: Pie) -> Result<(), RegisterError> {
        let reason = if !valid(name, &root) || self.entries.iter().any(|(n, _)| *n == name) {
            Some(PieFail::Denied)
        } else if self.entries.try_reserve(1).is_err() {
            Some(PieFail::OoM)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Failure {
                reason,
                value: (name, root),
            });
        }
        self.entries.push((name, root));
        Ok(())
    }
    pub(crate) fn freeze(self) -> Result<Frozen, FreezeError> {
        if self.entries.iter().any(|(name, pie)| !valid(*name, pie)) {
            return Err(Failure {
                reason: PieFail::Denied,
                value: self,
            });
        }
        Ok(Frozen {
            entries: self.entries,
        })
    }
}
