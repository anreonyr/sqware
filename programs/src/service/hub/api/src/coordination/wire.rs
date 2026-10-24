use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::Message;

use super::prepare::Prepare;
use super::release::Release;
use super::{PREPARE, RELEASE};


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Prepare(Prepare),
    Release(Release),
}

impl Wire {
    pub const LEN: usize = if Prepare::LEN > Release::LEN { Prepare::LEN } else { Release::LEN };

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            PREPARE => {
                let (value, end) = Prepare::fetch_at(bytes, 0)?;
                (end == bytes.len() && value.op == PREPARE
                    && valid_identity(value.hub, value.serial, value.execution))
                    .then_some(Self::Prepare(value))
            }
            RELEASE => {
                let (value, end) = Release::fetch_at(bytes, 0)?;
                (end == bytes.len() && value.op == RELEASE
                    && valid_identity(value.hub, value.serial, value.execution))
                    .then_some(Self::Release(value))
            }
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Prepare(value) if value.op == PREPARE
                && valid_identity(value.hub, value.serial, value.execution) => value.store_at(out, 0),
            Self::Release(value) if value.op == RELEASE
                && valid_identity(value.hub, value.serial, value.execution) => value.store_at(out, 0),
            _ => None,
        }
    }

    pub fn back(&self) -> PieToken {
        match self {
            Self::Prepare(value) => value.back,
            Self::Release(value) => value.back,
        }
    }
}

fn valid_identity(hub: TaskId, serial: u64, execution: TaskId) -> bool {
    hub != TaskId::new(0) && serial != 0 && execution != TaskId::new(0)
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
