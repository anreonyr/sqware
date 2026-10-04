use alloc::string::String;
use env::PieToken;
use env::wire::Span as _;
#[derive(Clone, env::Frame)]
#[frame(len = 64)]
pub struct Request {
    pub account: String,
    pub back: PieToken,
}
impl Request {
    pub fn take(bytes: &[u8]) -> Option<Self> {
        let (value, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len() && crate::common::name::valid(&value.account)).then_some(value)
    }
}
