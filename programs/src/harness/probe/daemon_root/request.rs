use env::wire::Span as _;
use env::{PieToken, TaskId};

/// The controller's private request after it observed the old target and its real heartbeat.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Request {
    pub old_task: TaskId,
    pub old_helper: TaskId,
    pub before: u64,
    pub ack: PieToken,
}

impl Request {
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (request, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}
