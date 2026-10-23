use env::wire::Span as _;
use env::TaskId;

/// The controller's private request after it observed the old RTC task and a real time.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 16)]
pub struct Request {
    pub old_task: TaskId,
    pub before: u64,
}

impl Request {
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (request, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}
