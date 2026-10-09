use crate::work::unit::gate::{self, AnyPie};
use crate::work::unit::task::{Task, TaskTag};
use alloc::vec::Vec;
use core::fmt;
use env::wire::Field;
use env::{Entry, Name, PieFail};

pub(crate) mod boot;

pub(crate) struct Failure<T> {
    pub reason: PieFail,
    pub value: T,
}
impl<T> Failure<T> {
    pub(crate) fn into_parts(self) -> (PieFail, T) {
        (self.reason, self.value)
    }
}
impl<T> fmt::Debug for Failure<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(f)
    }
}
pub(crate) type RegisterError = Failure<(Name, AnyPie)>;
pub(crate) type FreezeError = Failure<Registry>;
pub(crate) type GrantError = Failure<Frozen>;

#[derive(Default)]
pub(crate) struct Registry {
    entries: Vec<(Name, AnyPie)>,
}
pub(crate) struct Frozen {
    entries: Vec<(Name, AnyPie)>,
}

fn valid(name: Name, pie: &AnyPie) -> bool {
    let mut bytes = [0u8; env::NAME_LEN];
    name.store(&mut bytes);
    Name::fetch(&bytes) == Some(name)
        && name.kind() == pie.kind()
        && pie.alive()
        && pie.sire().is_none()
        && pie.heir().is_none()
        && !pie.permission().is_empty()
}

impl Registry {
    pub(crate) fn register(&mut self, name: Name, root: AnyPie) -> Result<(), RegisterError> {
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

impl Frozen {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn grant(self, target: &Task) -> Result<Vec<Entry>, GrantError> {
        let closed = target.gate.lock();
        if *closed || target.tag() != TaskTag::Held || self.entries.iter().any(|(n, p)| !valid(*n, p)) {
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
        let mut pies = target.pies.lock();
        if pies.try_reserve(self.entries.len()).is_err() {
            return Err(Failure {
                reason: PieFail::OoM,
                value: self,
            });
        }
        for (name, root) in self.entries {
            ledger.push(Entry::new(name, root.kind(), root.token()));
            pies.push(root);
        }
        drop(pies);
        gate::changed(target);
        drop(closed);
        let _ = crate::work::room::messenger::signal(
            crate::work::room::messenger::WakeKey::Capabilities {
                task: target.ident.id,
            },
        );
        Ok(ledger)
    }
}
