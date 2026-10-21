use env::TaskId;
use env::wire::Span as _;
use wire::Message;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 18)]
pub struct Frame {
    pub version: u8,
    pub role: u8,
    pub provider: TaskId,
    pub system: TaskId,
}

const _: () = assert!(Frame::LEN == 2 + <TaskId as env::wire::Field>::WIDTH * 2);

impl Message for Frame {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
