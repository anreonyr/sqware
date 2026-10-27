//! 收进来的一问。

use env::wire::Span as _;
use wire::Message;

use super::{DISABLE, ENABLE, PLACE, Place, RELEASE, Toggle};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Place(Place),
    Enable(Toggle),
    Disable(Toggle),
    Release(Toggle),
}

impl Wire {
    pub const LEN: usize = Place::LEN;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            PLACE => {
                let (value, end) = <Place as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Place(value))
            }
            ENABLE => {
                let (value, end) = <Toggle as env::wire::Span>::fetch_at(bytes, 0)?;
                (value.op == ENABLE && end == bytes.len()).then_some(Self::Enable(value))
            }
            DISABLE => {
                let (value, end) = <Toggle as env::wire::Span>::fetch_at(bytes, 0)?;
                (value.op == DISABLE && end == bytes.len()).then_some(Self::Disable(value))
            }
            RELEASE => {
                let (value, end) = <Toggle as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Release(value))
            }
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Place(value) if value.op == PLACE => value.store_at(out, 0),
            Self::Enable(value) if value.op == ENABLE => value.store_at(out, 0),
            Self::Disable(value) if value.op == DISABLE => value.store_at(out, 0),
            Self::Release(value) if value.op == RELEASE => value.store_at(out, 0),
            _ => None,
        }
    }
}

impl Message for Wire {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Wire::store(self, out)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Wire::take(bytes)
    }
}
