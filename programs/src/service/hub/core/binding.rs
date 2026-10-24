//! One device claim and its logical resource responsibility.

use env::{PieToken, TaskId};
use hub_api::{BindingId, BindingState, Close, DeviceId, Failure};

pub struct Binding {
    pub id: BindingId,
    pub claim: u64,
    pub device: DeviceId,
    pub execution: TaskId,
    pub sensor: PieToken,
    pub state: BindingState,
    /// 本次责任的理由（Fault 可覆盖 Normal，不反向恢复）。
    pub reason: Close,
    pub failure: Option<Failure>,
}

impl Binding {
    pub fn new(
        id: BindingId,
        claim: u64,
        device: DeviceId,
        execution: TaskId,
        sensor: PieToken,
    ) -> Self {
        Self {
            id,
            claim,
            device,
            execution,
            sensor,
            state: BindingState::Preparing,
            reason: Close::Normal,
            failure: None,
        }
    }

    /// 这一笔此刻还开着吗（未结清）。
    pub fn open(&self) -> bool {
        self.state != BindingState::Closed
    }

    pub fn close(&mut self, reason: Close) {
        if !self.open() {
            return;
        }
        if self.state != BindingState::Closing {
            self.state = BindingState::Closing;
        }
        if reason == Close::Fault {
            self.reason = Close::Fault;
        }
    }
}
