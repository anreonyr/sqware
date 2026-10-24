use env::wire::Span as _;
use wire::Message;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Said {
    pub status: u8,
    pub iommu: env::TaskId,
}

impl Message for Said {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
