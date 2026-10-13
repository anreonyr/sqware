//! Trusted export of a declared namespace binding under an authorized identity name.

use super::{IdentityName, IdentityObject, OperatorEntryRef};
use env::PieToken;
use wire::Message;
use wire::Span as _;

pub use super::interface::{EXPORT_BACK as BACK, EXPORT_ENTRY as ENTRY};
pub const OK: u8 = wire::OK;
pub const DENIED: u8 = 1;
pub const BAD: u8 = 2;
pub const FULL: u8 = 3;

#[derive(Clone, env::Frame)]
#[frame(len = 128)]
pub struct Request {
    pub object: IdentityObject,
    pub alias: IdentityName,
    pub binding: IdentityName,
    pub entry: OperatorEntryRef,
    pub back: PieToken,
}
impl Request {
    pub fn valid(&self) -> bool {
        self.object.authority().get() != 0
            && crate::operator::name::valid(self.alias.as_str())
            && crate::operator::name::valid(self.binding.as_str())
            && self.entry.operator.get() != 0
            && (!self.entry.root || self.entry.id == 0)
    }
}
impl Message for Request {
    type In = (Self, bool);
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self::In> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        Some((request, end == bytes.len()))
    }
}

#[derive(Clone, Copy, env::Frame)]
#[frame(len = 8)]
pub struct Response {
    pub status: u8,
}
impl Response {
    pub const fn accepted() -> Self {
        Self { status: OK }
    }
    pub const fn rejected(status: u8) -> Self {
        Self { status }
    }
}
impl Message for Response {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (response, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(response)
    }
}

pub fn reply_to(request: &(Request, bool)) -> PieToken {
    request.0.back
}

#[mold::contract(request = Request, response = Response, mark = BACK, back = reply_to)]
pub struct Call;
