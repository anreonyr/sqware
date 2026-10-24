use env::wire::Span as _;
use env::{PieToken, TaskId};

use super::api;

/// One real heartbeat request carrying its authenticated reply hole.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ask {
    pub op: u8,
    pub back: PieToken,
    pub target: TaskId,
}

impl Ask {
    pub fn of(back: PieToken, target: TaskId) -> Self {
        Self {
            op: api::ASK,
            back,
            target,
        }
    }

    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (ask, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(ask)
    }
}
