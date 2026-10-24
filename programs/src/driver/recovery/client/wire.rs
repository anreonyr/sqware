use env::wire::Span as _;
use wire::Message;

use super::{Observe, Supply, OBSERVE, SUPPLY};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Observe(Observe),
    Supply(Supply),
}

impl Wire {
    pub const LEN: usize = Supply::LEN;

    pub fn identity(&self) -> hub_api::BindingId {
        let (hub, serial) = match self {
            Self::Observe(frame) => (frame.hub, frame.serial),
            Self::Supply(frame) => (frame.hub, frame.serial),
        };
        hub_api::BindingId { hub, serial }
    }
}

impl Message for Wire {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Observe(frame) if frame.op == OBSERVE => frame.store_at(out, 0),
            Self::Supply(frame) if frame.op == SUPPLY => frame.store_at(out, 0),
            _ => None,
        }
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        match bytes.first().copied()? {
            OBSERVE => {
                let (frame, end) = Observe::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Observe(frame))
            }
            SUPPLY => {
                let (frame, end) = Supply::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Supply(frame))
            }
            _ => None,
        }
    }
}
