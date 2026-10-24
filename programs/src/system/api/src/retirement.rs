//! Generic retirement request: the provider alone decides what must settle.

use env::{PieToken, TaskId};
use env::wire::Span as _;
use wire::Message;

pub const ASK: u8 = 0;
pub const PENDING: u8 = 1;
pub const DONE: u8 = 2;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 24)]
pub struct Retirement {
    pub state: u8,
    pub target: TaskId,
    pub back: PieToken,
}

impl Retirement {
    pub const fn ask(back: PieToken) -> Self {
        Self::ask_for(TaskId::new(0), back)
    }

    pub const fn ask_for(target: TaskId, back: PieToken) -> Self {
        Self { state: ASK, target, back }
    }

    pub const fn pending() -> Self {
        Self::pending_for(TaskId::new(0))
    }

    pub const fn pending_for(target: TaskId) -> Self {
        Self {
            state: PENDING,
            target,
            back: PieToken::NONE,
        }
    }

    pub const fn done() -> Self {
        Self::done_for(TaskId::new(0))
    }

    pub const fn done_for(target: TaskId) -> Self {
        Self {
            state: DONE,
            target,
            back: PieToken::NONE,
        }
    }

    pub const fn settled(&self) -> bool {
        self.state == DONE
    }
}

impl Message for Retirement {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}

#[mold::interface(id = "sqware.system.retirement.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "back", legacy = "system-retirement-back")]
        Back,
    }
}

pub use interface::{BACK, CHANNELS as DECLARATIONS, REGISTRY};

fn reply_to(request: &Retirement) -> PieToken {
    request.back
}

#[mold::contract(request = Retirement, response = Retirement, mark = BACK, back = reply_to)]
pub struct Call;
