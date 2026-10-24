use env::{PieToken, wire::Span as _};
use wire::Message;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Scan {
    pub from: u32,
    pub back: PieToken,
}

impl Message for Scan {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        if self.from > 65536 { return None; }
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (frame, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len() && frame.from <= 65536).then_some(frame)
    }
}
