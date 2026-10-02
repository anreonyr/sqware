//! Private Hub → Control device-identity installation, never an Operator tile.
use env::{Mark, PieToken, TaskId};

use crate::service::identity::CoalitionId;
use crate::wire::message::Message;
use env::wire::Span as _;

pub const ENTRY: Mark = Mark::of("hub-activate");
pub const BACK: Mark = Mark::of("hub-activate-back");

#[derive(Clone, Copy, env::Frame)]
pub struct Activate {
    pub task: TaskId,
    pub coalition: CoalitionId,
    pub back: PieToken,
}

impl Message for Activate {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        (self.task.get() != 0 && self.coalition.authority.get() != 0)
            .then_some(())?;
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (ask, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len() && ask.task.get() != 0 && ask.coalition.authority.get() != 0)
            .then_some(ask)
    }
}
