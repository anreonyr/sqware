use env::wire::Span as _;
use wire::Message;

/// Device protocol offsets and the IOMMU-accepted address for one activation.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 48)]
pub struct Coords {
    pub bar_pages: u32,
    pub common: u32,
    pub notify: u32,
    pub multiplier: u32,
    pub device: u32,
    pub iova: u64,
    pub bar: env::PieToken,
    pub bytes: u64,
}

impl Message for Coords {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> { self.store_at(out, 0) }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
