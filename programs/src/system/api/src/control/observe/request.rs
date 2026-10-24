use env::wire::Span as _;
use env::PieToken;
use wire::Message;

/// One page position in the supervised target list.
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 12)]
pub struct Request {
    pub index: u32,
    pub back: PieToken,
}

impl Message for Request {
    type In = Request;
    type Buf = [u8; Request::LEN];
    const EMPTY: Self::Buf = [0; Request::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Request> {
        let (request, end) = Request::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}
