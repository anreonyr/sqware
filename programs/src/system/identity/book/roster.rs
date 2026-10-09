use env::TaskId;
use system_api::identity::Binding;
use system_api::identity::Fail;
use system_api::identity::Install;
use system_api::identity::Match;
use system_api::identity::Selector;
use system_api::identity::Subject;

use super::{IdentityBook, limits};

pub(super) struct BindingRow {
    pub(super) task: TaskId,
    pub(super) binding: Binding,
}

/// What the narrowed subject replaces in the sender's own binding.
#[derive(Clone, Copy)]
pub enum Anchor {
    /// The origin stays; only the current selection narrows.
    Keep,
    /// The narrowed subject becomes the origin and the current selection.
    Move,
}

impl IdentityBook {
    pub fn resolve(&self, task: TaskId) -> Option<Binding> {
        self.bindings
            .iter()
            .find(|b| b.task == task)
            .map(|b| b.binding)
    }

    fn validate(&self, s: Subject) -> Result<(), Fail> {
        self.principal(s.principal)?;
        for &c in s.coalitions.as_slice() {
            if !self.amid(s.principal, c)? {
                return Err(Fail::NotEligible);
            }
        }
        Ok(())
    }

    fn narrow(&self, base: Subject, requested: Subject) -> Result<(), Fail> {
        self.validate(requested)?;
        if !self.heir(base.principal, requested.principal)?
            || requested
                .coalitions
                .as_slice()
                .iter()
                .any(|c| !base.coalitions.contains(*c))
        {
            return Err(Fail::NotNarrower);
        }
        Ok(())
    }

    pub fn matches(&self, task: TaskId, selector: Selector) -> Result<Match, Fail> {
        match selector {
            Selector::Exact(p) | Selector::DescendantOf(p) => {
                self.principal(p)?;
            }
            Selector::MemberOf(c) => {
                self.coalition(c)?;
            }
        }
        let Some(b) = self.resolve(task) else {
            return Ok(Match::Unbound);
        };
        let yes = match selector {
            Selector::Exact(p) => b.current.principal == p,
            Selector::DescendantOf(p) => self.heir(p, b.current.principal)?,
            Selector::MemberOf(c) => {
                b.current.coalitions.contains(c) && self.amid(b.current.principal, c)?
            }
        };
        Ok(if yes { Match::Yes } else { Match::No })
    }

    pub fn same(&self, a: TaskId, b: TaskId) -> Match {
        match (self.resolve(a), self.resolve(b)) {
            (Some(a), Some(b)) => {
                if a.current.principal == b.current.principal {
                    Match::Yes
                } else {
                    Match::No
                }
            }
            _ => Match::Unbound,
        }
    }

    pub fn bind(&mut self, from: TaskId, request: BindingRequest) -> Result<(), Fail> {
        let BindingRequest { task, install } = request;
        if from != self.installer {
            return Err(Fail::Denied);
        }
        let subject = match install {
            Install::Authorized(s) => {
                self.validate(s)?;
                s
            }
            Install::Inherit { parent } => self.resolve(parent).ok_or(Fail::Denied)?.current,
            Install::Restrict { parent, subject } => {
                self.narrow(self.resolve(parent).ok_or(Fail::Denied)?.current, subject)?;
                subject
            }
        };
        let binding = Binding {
            origin: subject,
            current: subject,
        };
        if let Some(b) = self.bindings.iter_mut().find(|b| b.task == task) {
            b.binding = binding;
        } else {
            if self.bindings.len() >= limits::MAX_BINDINGS {
                return Err(Fail::Full);
            }
            self.bindings.try_reserve(1).map_err(|_| Fail::Full)?;
            self.bindings.push(BindingRow { task, binding });
        }
        Ok(())
    }

    pub fn unbind(&mut self, from: TaskId, task: TaskId) -> Result<(), Fail> {
        if from != self.installer {
            return Err(Fail::Denied);
        }
        self.bindings.retain(|b| b.task != task);
        Ok(())
    }

    pub fn narrow_own(&mut self, from: TaskId, selection: Selection) -> Result<(), Fail> {
        let Selection { subject, anchor } = selection;
        let at = self
            .bindings
            .iter()
            .position(|b| b.task == from)
            .ok_or(Fail::Denied)?;
        self.narrow(self.bindings[at].binding.current, subject)?;
        match anchor {
            Anchor::Keep => self.bindings[at].binding.current = subject,
            Anchor::Move => {
                self.bindings[at].binding = Binding {
                    origin: subject,
                    current: subject,
                }
            }
        }
        Ok(())
    }

    pub fn waive(&mut self, from: TaskId) -> Result<(), Fail> {
        let b = self
            .bindings
            .iter_mut()
            .find(|b| b.task == from)
            .ok_or(Fail::Denied)?;
        b.binding.current = b.binding.origin;
        Ok(())
    }
}

pub struct BindingRequest {
    pub task: TaskId,
    pub install: Install,
}
pub struct Selection {
    pub subject: Subject,
    pub anchor: Anchor,
}

impl IdentityBook {
    /// A coalition manager may activate only its own, already eligible coalitions.
    /// Preserve attenuation and perform the entire set check before changing either snapshot.
    pub fn activate(&mut self, from: TaskId, request: (TaskId, system_api::identity::CoalitionSet)) -> Result<(), Fail> {
        let (task, coalitions) = request;
        if coalitions.is_empty() { return Err(Fail::Bad); }
        let manager = self.resolve(from).ok_or(Fail::Denied)?.current.principal;
        let binding = self.resolve(task).ok_or(Fail::Denied)?;
        if binding.origin != binding.current { return Err(Fail::NotNarrower); }
        let mut ids = [system_api::identity::CoalitionId::new(self.authority, 0); system_api::identity::limits::MAX_ACTIVE_COALITIONS];
        let mut n = binding.current.coalitions.len();
        ids[..n].copy_from_slice(binding.current.coalitions.as_slice());
        for coalition in coalitions.iter() {
            let index = self.coalition(coalition)?;
            if self.coalitions[index].manager != manager { return Err(Fail::NotManager); }
            if !self.amid(binding.current.principal, coalition)? { return Err(Fail::NotEligible); }
            if !ids[..n].contains(&coalition) {
                if n == ids.len() { return Err(Fail::Full); }
                ids[n] = coalition;
                n += 1;
            }
        }
        let subject = Subject::new(binding.current.principal, &ids[..n])?;
        let row = self.bindings.iter_mut().find(|row| row.task == task).ok_or(Fail::Denied)?;
        row.binding = Binding { origin: subject, current: subject };
        Ok(())
    }
}
