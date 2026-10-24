use env::{PieToken, TaskId};
use env::wire::Span as _;
use wire::Message;

use super::dma::{BindingState, Failure};

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 32)]
pub struct DmaSaid {
    pub status: u8,
    pub state: u8,
    pub party: u8,
    pub op: u8,
    pub code: u8,
    pub count: u8,
    pub serial: u64,
    pub source: TaskId,
    pub client: PieToken,
}

impl DmaSaid {
    pub const NONE: Self = Self {
        status: 0, state: 0, party: 0, op: 0, code: 0, count: 0, serial: 0,
        source: TaskId::new(0), client: PieToken::NONE,
    };

    pub const fn of(status: u8) -> Self {
        Self { status, ..Self::NONE }
    }

    pub const fn bound(serial: u64, state: BindingState) -> Self {
        Self { status: wire::OK, state: state.code(), serial, ..Self::NONE }
    }

    pub const fn progress(serial: u64, state: BindingState, failure: Option<Failure>) -> Self {
        let (party, op, code) = match failure {
            Some(failure) => (failure.party, failure.op, failure.code),
            None => (0, 0, 0),
        };
        Self { status: wire::OK, state: state.code(), party, op, code, serial, ..Self::NONE }
    }

    pub fn failure(&self) -> Option<Failure> {
        (self.party != 0).then_some(Failure { party: self.party, op: self.op, code: self.code })
    }
}

impl Message for DmaSaid {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
