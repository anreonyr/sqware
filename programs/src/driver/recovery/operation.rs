use env::{PieToken, TaskId};
use hub_api::{BindingState, Close, Failure};

use super::stage::Stage;

/// One owned binding operation, including every acquired hardware handle.
pub struct Operation {
    pub hub: TaskId,
    pub serial: u64,
    pub execution: TaskId,
    pub key: u64,
    pub first_failure: Option<Failure>,
    pub reason: Close,
    pub stage: Stage,
    pub bar: Option<PieToken>,
    pub handoff: Option<PieToken>,
    pub pages: u32,
    pub caps: (u32, u32, u32, u32),
    pub iova: u64,
}

impl Operation {
    pub fn new(
        hub: TaskId,
        serial: u64,
        execution: TaskId,
        key: u64,
    ) -> Self {
        Self {
            hub,
            serial,
            execution,
            key,
            first_failure: None,
            reason: Close::Normal,
            stage: Stage::Place,
            bar: None,
            handoff: None,
            pages: 0,
            caps: (0, 0, 0, 0),
            iova: 0,
        }
    }

    pub fn fail(&mut self, party: u8, op: u8, code: u8) {
        self.first_failure.get_or_insert(Failure { party, op, code });
    }

    pub fn state(&self) -> BindingState {
        match self.stage {
            Stage::Active => BindingState::Active,
            Stage::Done => BindingState::Closed,
            Stage::Invalidate
            | Stage::Disable
            | Stage::Stop
            | Stage::Quiesce
            | Stage::Release
            | Stage::Detach
            | Stage::CloseHandoff
            | Stage::Unplace => BindingState::Closing,
            _ => BindingState::Preparing,
        }
    }
}
