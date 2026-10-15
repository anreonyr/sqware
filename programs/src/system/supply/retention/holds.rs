use alloc::vec::Vec;
use env::{FlatReference, PieToken, TaskId};

use super::memory::Memory;
use super::{Close, DeviceRange, Failure, MemoryEffects, Phase, Registration, Scope};

pub struct Holds {
    scopes: Vec<Scope>,
    next: usize,
    closed: bool,
}

impl Holds {
    pub const fn new() -> Self {
        Self {
            scopes: Vec::new(),
            next: 1,
            closed: false,
        }
    }

    pub fn attach(&mut self, scope: Scope) -> Result<(), Failure> {
        if self.closed {
            return Err(Failure::Closed);
        }
        if self
            .scopes
            .iter()
            .any(|other| other.range.overlaps(scope.range))
        {
            return Err(Failure::Conflict);
        }
        if scope
            .platform
            .as_ref()
            .is_some_and(|platform| platform.range() != scope.range)
        {
            return Err(Failure::Denied);
        }
        self.scopes.try_reserve(1).map_err(|_| Failure::Full)?;
        self.scopes.push(scope);
        Ok(())
    }

    /// Reserve metadata before Acquire; after success the reference has exactly one owner.
    pub fn register(
        &mut self,
        registration: Registration,
        effects: &mut impl MemoryEffects,
    ) -> Result<usize, Failure> {
        let Registration {
            range,
            source,
            retention,
        } = registration;
        if self.closed {
            return Err(Failure::Closed);
        }
        if retention == PieToken::NONE {
            return Err(Failure::Denied);
        }
        let next = self.next.checked_add(1).ok_or(Failure::Full)?;
        let scope = self
            .scopes
            .iter_mut()
            .find(|scope| scope.range == range)
            .ok_or(Failure::Unknown)?;
        if scope.phase != Phase::Open {
            return Err(Failure::Closed);
        }
        scope.memory.try_reserve(1).map_err(|_| Failure::Full)?;
        let reference = effects.acquire(source, retention)?;
        let id = self.next;
        scope.memory.push(Memory {
            id,
            reference,
            published: false,
        });
        self.next = next;
        Ok(id)
    }

    /// Mark reachable before executing publication: even a partial failure needs isolation.
    /// The callback must not publish any other memory or retain a path around this gate.
    pub fn publish(
        &mut self,
        id: usize,
        publish: impl FnOnce(FlatReference) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        if self.closed {
            return Err(Failure::Closed);
        }
        let scope = self
            .scopes
            .iter_mut()
            .find(|scope| scope.memory.iter().any(|memory| memory.id == id))
            .ok_or(Failure::Unknown)?;
        if scope.phase != Phase::Open {
            return Err(Failure::Closed);
        }
        if scope.platform.is_none() {
            return Err(Failure::Unavailable);
        }
        let memory = scope
            .memory
            .iter_mut()
            .find(|memory| memory.id == id)
            .ok_or(Failure::Unknown)?;
        if memory.published {
            return Err(Failure::Conflict);
        }
        memory.published = true;
        let result = publish(memory.reference);
        if let Err(failure) = result {
            scope.phase = Phase::Closing(Close::Fault);
            scope.failure = Some(failure);
        }
        result
    }

    pub fn close(&mut self, range: DeviceRange, reason: Close) -> Result<(), Failure> {
        let scope = self
            .scopes
            .iter_mut()
            .find(|scope| scope.range == range)
            .ok_or(Failure::Unknown)?;
        Self::begin_close(scope, reason);
        Ok(())
    }

    /// Death closes publication immediately, without waiting for ordinary task cleanup.
    pub fn offline(&mut self, controller: TaskId) {
        for scope in &mut self.scopes {
            if scope.controller == controller {
                Self::begin_close(scope, Close::Fault);
            }
        }
    }

    pub fn close_all(&mut self) {
        self.closed = true;
        for scope in &mut self.scopes {
            Self::begin_close(scope, Close::Normal);
        }
    }

    fn begin_close(scope: &mut Scope, reason: Close) {
        match (scope.phase, reason) {
            (Phase::Open, _) | (Phase::Closing(Close::Normal), Close::Fault) => {
                scope.phase = Phase::Closing(reason);
            }
            _ => {}
        }
    }

    /// No table lock or failed-service RPC is held across platform progress.
    /// Failure in one scope does not prevent independent scopes from settling.
    pub fn retry(&mut self, effects: &mut impl MemoryEffects) -> bool {
        let mut failed = false;
        for scope in &mut self.scopes {
            let previous = scope.failure;
            Self::progress(scope, effects);
            failed |= scope.failure.is_some() && scope.failure != previous;
        }
        failed
    }

    fn progress(scope: &mut Scope, effects: &mut impl MemoryEffects) {
        let Phase::Closing(reason) = scope.phase else {
            if scope.phase == Phase::Stopped {
                Self::release(scope, effects);
            }
            return;
        };
        if scope.memory.iter().any(|memory| memory.published) {
            let Some(platform) = scope.platform.as_mut() else {
                scope.failure = Some(Failure::Unavailable);
                return;
            };
            let result = match reason {
                Close::Normal => platform.synchronize(),
                Close::Fault => platform.isolate(),
            };
            if let Err(failure) = result {
                scope.failure = Some(failure);
                return;
            }
        }
        scope.phase = Phase::Stopped;
        Self::release(scope, effects);
    }

    fn release(scope: &mut Scope, effects: &mut impl MemoryEffects) {
        scope.failure = None;
        let mut at = 0;
        while at < scope.memory.len() {
            match effects.release(scope.memory[at].reference) {
                Ok(()) => {
                    scope.memory.remove(at);
                }
                Err(failure) => {
                    scope.failure = Some(failure);
                    at += 1;
                }
            }
        }
    }

    pub fn controllers(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.scopes
            .iter()
            .filter(|scope| scope.phase == Phase::Open)
            .map(|scope| scope.controller)
    }

    pub fn occupied(
        &self,
    ) -> impl Iterator<Item = (DeviceRange, TaskId, Phase, usize, Option<Failure>)> + '_ {
        self.scopes
            .iter()
            .filter(|scope| !scope.memory.is_empty())
            .map(|scope| {
                (
                    scope.range,
                    scope.controller,
                    scope.phase,
                    scope.memory.len(),
                    scope.failure,
                )
            })
    }
}
