/// Private startup image transfer; the seed is in the recipient table.
pub const IMAGE_MARK: env::Mark = env::Mark::of("unit-image");
#[derive(env::Frame, Clone, Copy)]
pub struct ImageSupplyFrame {
    pub seed: env::PieToken,
    pub length: u64,
}

impl wire::Message for ImageSupplyFrame {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        env::wire::Span::store_at(self, out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::LEN {
            return None;
        }
        env::wire::Span::fetch_at(bytes, 0).map(|(frame, _)| frame)
    }
}
