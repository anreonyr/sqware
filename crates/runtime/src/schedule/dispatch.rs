use super::{Cursor, Progress, RunError};

pub struct Invocation<K> {
    pub key: K,
    pub cursor: Cursor,
}
pub struct Completion<K, E> {
    pub invocation: Invocation<K>,
    pub result: Result<Progress, RunError<E>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchError {
    Busy,
    Exhausted,
    NoResult,
}
enum State<K, E> {
    Idle,
    Selected(Invocation<K>),
    Running,
    Finished(Completion<K, E>),
}

/// One bounded dispatch round; selection and completion cannot overlap.
pub struct Dispatch<K, E> {
    remaining: usize,
    state: State<K, E>,
}
impl<K, E> Dispatch<K, E> {
    pub fn new() -> Self {
        Self { remaining: 0, state: State::Idle }
    }
    pub fn begin(&mut self, budget: usize) -> Result<(), DispatchError> {
        if !self.is_idle() { return Err(DispatchError::Busy); }
        self.remaining = budget;
        Ok(())
    }
    pub fn remaining(&self) -> usize { self.remaining }
    pub fn select(&mut self, invocation: Invocation<K>) -> Result<(), DispatchError> {
        if !self.is_idle() { return Err(DispatchError::Busy); }
        if self.remaining == 0 { return Err(DispatchError::Exhausted); }
        self.state = State::Selected(invocation);
        Ok(())
    }
    /// Charge a skipped queue item against this round's budget.
    pub fn skip(&mut self) -> Result<(), DispatchError> {
        if !self.is_idle() { return Err(DispatchError::Busy); }
        self.remaining = self.remaining.checked_sub(1).ok_or(DispatchError::Exhausted)?;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), DispatchError> {
        if !self.is_idle() { return Err(DispatchError::Busy); }
        self.remaining = 0;
        Ok(())
    }
    /// Consume the invocation and its result together.
    pub fn take_result(&mut self) -> Result<Completion<K, E>, DispatchError> {
        if !matches!(self.state, State::Finished(_)) { return Err(DispatchError::NoResult); }
        let State::Finished(completion) = core::mem::replace(&mut self.state, State::Idle) else { unreachable!() };
        Ok(completion)
    }
    pub(crate) fn is_idle(&self) -> bool { matches!(self.state, State::Idle) }
    pub(crate) fn has_selected(&self) -> bool { matches!(self.state, State::Selected(_)) }
    pub(crate) fn take_selected(&mut self) -> Option<Invocation<K>> {
        if !self.has_selected() { return None; }
        let State::Selected(invocation) = core::mem::replace(&mut self.state, State::Running) else { unreachable!() };
        Some(invocation)
    }
    pub(crate) fn restore_selected(&mut self, invocation: Invocation<K>) -> Result<(), DispatchError> {
        if !matches!(self.state, State::Running) { return Err(DispatchError::Busy); }
        self.state = State::Selected(invocation);
        Ok(())
    }
    pub(crate) fn complete(&mut self, completion: Completion<K, E>) -> Result<(), DispatchError> {
        if !matches!(self.state, State::Running) { return Err(DispatchError::Busy); }
        self.remaining = self.remaining.checked_sub(1).ok_or(DispatchError::Exhausted)?;
        self.state = State::Finished(completion);
        Ok(())
    }
}
impl<K, E> Default for Dispatch<K, E> {
    fn default() -> Self { Self::new() }
}
