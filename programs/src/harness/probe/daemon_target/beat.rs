use env::wire::Span as _;
use env::TaskId;

use super::api;

/// The target's own task, its live helper task and the real clock at reply time.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Beat {
    pub op: u8,
    pub target: TaskId,
    pub helper: TaskId,
    pub now: u64,
}

impl Beat {
    pub fn of(target: TaskId, helper: TaskId, now: u64) -> Self {
        Self {
            op: api::BEAT,
            target,
            helper,
            now,
        }
    }

    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (beat, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(beat)
    }
}
