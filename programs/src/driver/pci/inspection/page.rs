use wire::Message;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Page {
    pub status: u8,
    pub next: u32,
    pub requester: u16,
    pub count: u8,
    pub end: u8,
    pub bytes: [u8; 256],
}

impl Page {
    pub const LEN: usize = 265;
    pub const fn of(status: u8) -> Self {
        Self { status, next: 0, requester: 0, count: 0, end: 0, bytes: [0; 256] }
    }
    fn valid(&self) -> bool {
        if self.status != wire::OK { return self.count == 0 && self.end == 0; }
        self.next <= 65536 && self.count <= 1
            && matches!(self.end, super::CAPACITY | super::WORK_LIMIT | super::EOF)
            && (self.count == 0 || self.next > u32::from(self.requester))
            && (self.count != 0 || self.end != super::CAPACITY)
    }
}

impl Message for Page {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        if !self.valid() { return None; }
        let out = out.get_mut(..Self::LEN)?;
        out[0] = self.status;
        out[1..5].copy_from_slice(&self.next.to_le_bytes());
        out[5..7].copy_from_slice(&self.requester.to_le_bytes());
        out[7] = self.count;
        out[8] = self.end;
        out[9..].copy_from_slice(&self.bytes);
        Some(Self::LEN)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::LEN { return None; }
        let page = Self {
            status: bytes[0],
            next: u32::from_le_bytes(bytes[1..5].try_into().ok()?),
            requester: u16::from_le_bytes(bytes[5..7].try_into().ok()?),
            count: bytes[7], end: bytes[8], bytes: bytes[9..].try_into().ok()?,
        };
        page.valid().then_some(page)
    }
}
