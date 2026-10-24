use env::{PieToken, TaskId};
use hub_api::{BindingState, Close, Failure};

use super::exchange::Exchange;
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
    pub closing: bool,
    pub exchange: Option<Exchange>,
    pub bar: Option<PieToken>,
    pub handoff: Option<(PieToken, ipc::time::Deadline)>,
    pub client: Option<(PieToken, PieToken)>,
    pub source: Option<(PieToken, u32)>,
    pub pages: u32,
    pub caps: (u32, u32, u32, u32),
    pub iova: u64,
    pub bytes: u64,
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
            closing: false,
            exchange: None,
            bar: None,
            handoff: None,
            client: None,
            source: None,
            pages: 0,
            caps: (0, 0, 0, 0),
            iova: 0,
            bytes: 0,
        }
    }

    pub fn fail(&mut self, party: u8, op: u8, code: u8) {
        self.first_failure.get_or_insert(Failure { party, op, code });
    }

    pub fn state(&self) -> BindingState {
        if self.closing {
            return BindingState::Closing;
        }
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
