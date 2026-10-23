use alloc::string::String;
use env::wire::Span as _;
use wire::Message;

use crate::control::frame::{DENIED, OK};

/// One supervised target observation.
///
/// `end` is the explicit list terminator; `present` says whether the row exists.
/// EOF is never expressed as a fabricated dead row.
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 48)]
pub struct Reply {
    pub status: u8,
    pub index: u32,
    pub present: bool,
    pub end: bool,
    pub run_requested: bool,
    pub state: u8,
    pub name: String,
}

impl Reply {
    pub const fn end(index: u32) -> Self {
        Self {
            status: OK,
            index,
            present: false,
            end: true,
            run_requested: false,
            state: 0,
            name: String::new(),
        }
    }

    pub const fn denied(index: u32) -> Self {
        Self {
            status: DENIED,
            index,
            present: false,
            end: false,
            run_requested: false,
            state: 0,
            name: String::new(),
        }
    }
}

impl Message for Reply {
    type In = Reply;
    type Buf = [u8; Reply::LEN];
    const EMPTY: Self::Buf = [0; Reply::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Reply> {
        let (reply, end) = Reply::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(reply)
    }
}
