use env::{TaskId, UnitFail};

use crate::machine::sync::{Level, SpinLock};

use super::mode::Mode;
use super::state::State;

pub(crate) struct ExitPolicy {
    state: SpinLock<State>,
}

impl ExitPolicy {
    pub(crate) const fn new() -> Self {
        Self {
            state: SpinLock::new_level(
                Level::L3,
                State {
                    mode: Mode::Dormant,
                    deaths: 0,
                },
            ),
        }
    }

    pub(crate) fn arm(&self, owner: TaskId) -> Result<(), UnitFail> {
        let mut state = self.state.lock();
        if state.deaths != 0 {
            return Err(UnitFail::Denied);
        }
        match state.mode {
            Mode::Dormant => {
                state.mode = Mode::Armed { owner };
                Ok(())
            }
            Mode::Armed { owner: current } if current == owner => Ok(()),
            Mode::Armed { .. } => Err(UnitFail::Busy),
            Mode::Tripped => Err(UnitFail::Denied),
        }
    }

    pub(crate) fn disarm(&self, owner: TaskId) -> Result<(), UnitFail> {
        let mut state = self.state.lock();
        if state.deaths != 0 {
            return Err(UnitFail::Denied);
        }
        match state.mode {
            Mode::Armed { owner: current } if current == owner => {
                state.mode = Mode::Dormant;
                Ok(())
            }
            _ => Err(UnitFail::Denied),
        }
    }

    pub(crate) fn protected(&self) -> bool {
        !matches!(self.state.lock().mode, Mode::Dormant)
    }

    /// Every death claim returns `true` once armed or tripped; only `Dormant`
    /// allows progress. `alarm` owns the single-claim rule, not this policy.
    pub(crate) fn trip(&self) -> bool {
        let mut state = self.state.lock();
        state.deaths = state.deaths.saturating_add(1);
        match state.mode {
            Mode::Dormant => false,
            Mode::Armed { .. } => {
                state.mode = Mode::Tripped;
                true
            }
            Mode::Tripped => true,
        }
    }
}
