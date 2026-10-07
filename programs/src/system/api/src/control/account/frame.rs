use alloc::string::String;
use env::PieToken;
use env::wire::Span as _;
use wire::message::Message;

#[derive(Clone, env::Frame)]
#[frame(len = 64)]
pub struct Request {
    pub account: String,
    pub back: PieToken,
}
impl Request {
    pub fn take(bytes: &[u8]) -> Option<Self> {
        let (value, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len() && crate::operator::name::valid(&value.account)).then_some(value)
    }
}
impl Message for Request {
    type In = (Self, bool);
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }

    fn fetch(bytes: &[u8]) -> Option<Self::In> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        Some((request, end == bytes.len()))
    }
}
