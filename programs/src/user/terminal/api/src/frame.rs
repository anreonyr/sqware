use wire::message::Message;
use env::wire::Span as _;
use env::{PieToken, TaskId};

pub use super::marks::ENTRY;
pub use super::marks::AUTHORITY;
pub use super::marks::BACK;
pub use super::marks::INPUT;
pub use super::marks::OUTPUT;
pub use super::marks::CONTROL;
pub const ATTACH: u8 = 1;
pub const FOREGROUND: u8 = 2;
pub const DETACH: u8 = 3;
pub const ECHO_OFF: u8 = 4;
pub const ECHO_ON: u8 = 5;
pub const INTERRUPT: u8 = 1;
pub const DATA: u8 = 1;
pub const EOF: u8 = 2;
pub const MAX: usize = 256;

#[derive(Clone, Copy, env::Frame)]
pub struct Command {
    pub op: u8,
    pub task: TaskId,
    pub authority: PieToken,
    pub back: PieToken,
}

#[derive(Clone, Copy, env::Frame)]
pub struct Reply {
    pub status: u8,
    pub authority: PieToken,
}

#[derive(Clone, env::Frame)]
pub struct Input {
    pub kind: u8,
    n: u32,
    #[frame(count = n, fill = 0)]
    bytes: [u8; MAX],
}
impl Input {
    pub fn data(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX {
            return None;
        }
        let mut input = Self {
            kind: DATA,
            n: bytes.len() as u32,
            bytes: [0; MAX],
        };
        input.bytes[..bytes.len()].copy_from_slice(bytes);
        Some(input)
    }
    pub fn eof() -> Self {
        Self {
            kind: EOF,
            n: 0,
            bytes: [0; MAX],
        }
    }
    fn valid(&self) -> bool {
        match self.kind {
            DATA => self.n > 0 && self.n as usize <= MAX,
            EOF => self.n == 0,
            _ => false,
        }
    }
    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.n = 0;
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.n as usize]
    }
}
impl Message for Input {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.valid().then_some(())?;
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (input, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len() && input.valid()).then_some(input)
    }
}
