#![no_std]
use env::{PieToken, TaskId};
pub use stream::{DEFAULT_CAPACITY, Direction, HEADER_SIZE, Header, MAX_CAPACITY, Read, Write};
pub const ENTRY: env::Mark = env::Mark::of("pipe-entry");
pub const BACK: env::Mark = env::Mark::of("pipe-back");
pub const LEASE: env::Mark = env::Mark::of("pipe-lease");
pub const DATA: env::Mark = env::Mark::of("pipe-data");
pub const DIR: &str = "/svc/pipe/create";
pub const CREATE: u8 = 1;
pub const BIND: u8 = 2;
pub const RELEASE: u8 = 3;
pub const CLOSE: u8 = 5;
pub const READ: u8 = 1;
pub const WRITE: u8 = 2;
pub const READ_BIT: env::Bit = env::Bit::FIRST;
pub const WRITE_BIT: env::Bit = match env::Bit::of(1) {
    Some(bit) => bit,
    None => unreachable!(),
};
pub const DEMAND_BIT: env::Bit = match env::Bit::of(2) {
    Some(bit) => bit,
    None => unreachable!(),
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Denied,
    Dead,
    Full,
    Invalid,
    Protocol,
    Closed,
    BrokenPipe,
}
impl Fail {
    pub const fn code(self) -> u8 {
        match self {
            Self::Denied => 1,
            Self::Dead => 2,
            Self::Full => 3,
            Self::Invalid => 4,
            Self::Protocol => 5,
            Self::Closed => 6,
            Self::BrokenPipe => 7,
        }
    }
    pub fn of(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Denied,
            2 => Self::Dead,
            3 => Self::Full,
            4 => Self::Invalid,
            5 => Self::Protocol,
            6 => Self::Closed,
            7 => Self::BrokenPipe,
            _ => return None,
        })
    }
}
impl From<stream::Fail> for Fail {
    fn from(fail: stream::Fail) -> Self {
        match fail {
            stream::Fail::Invalid => Self::Invalid,
            stream::Fail::Corrupt => Self::Protocol,
            stream::Fail::Closed => Self::Closed,
            stream::Fail::BrokenPipe => Self::BrokenPipe,
        }
    }
}
#[derive(Clone, Copy, env::Frame)]
pub struct Request {
    pub op: u8,
    pub id: u64,
    pub capacity: u64,
    pub target: TaskId,
    pub direction: u8,
    pub lease: PieToken,
    pub back: PieToken,
}
#[derive(Clone, Copy, env::Frame)]
pub struct Reply {
    pub status: u8,
    pub id: u64,
    pub capacity: u64,
    pub seed: PieToken,
}
fn reply_to(request: &Request) -> PieToken {
    request.back
}
#[mold::contract(request = Request, response = Reply, mark = BACK, back = reply_to)]
pub struct Call;
impl wire::Message for Request {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        env::wire::Span::store_at(self, out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = env::wire::Span::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
pub fn direction(raw: u8) -> Option<Direction> {
    match raw {
        READ => Some(Direction::Read),
        WRITE => Some(Direction::Write),
        _ => None,
    }
}
pub fn code(direction: Direction) -> u8 {
    match direction {
        Direction::Read => READ,
        Direction::Write => WRITE,
    }
}
pub struct Endpoint {
    pub id: u64,
    pub capacity: usize,
    pub seed: PieToken,
    pub direction: Direction,
}

impl wire::Message for Reply {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        env::wire::Span::store_at(self, out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = env::wire::Span::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
