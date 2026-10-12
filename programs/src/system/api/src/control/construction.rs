pub use super::marks::{CONSTRUCTION_BACK as BACK, CONSTRUCTION_ENTRY as ENTRY};
use crate::{
    identity::Subject,
    loader::Said,
};
use alloc::string::String;
use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CreateId {
    pub value: u64,
}
impl env::wire::Span for CreateId {
    const MAX: Option<usize> = <u64 as env::wire::Span>::MAX;
    fn store_at(&self, bytes: &mut [u8], at: usize) -> Option<usize> {
        self.value.store_at(bytes, at)
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (value, end) = u64::fetch_at(bytes, at)?;
        Some((Self { value }, end))
    }
}

pub const BUILD: u8 = 1;
pub const INSPECT: u8 = 2;
pub const CANCEL: u8 = 3;
pub const UNSEEN: u8 = 0;
pub const PENDING: u8 = 1;
pub const COMPLETED: u8 = 2;
pub const FAILED: u8 = 3;
pub const CANCELLED: u8 = 4;
pub const CLOSED: u8 = 5;
#[derive(Clone, Debug, PartialEq, Eq, env::Frame)]
#[frame(len = 800)]
pub struct UnitRef {
    pub name: String,
    pub stack: u64,
    pub count: u8,
    #[frame(count = count, fill = 0)]
    pub args: [u64; crate::loader::MAX_ARGS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, env::Frame)]
#[frame(len = 600)]
pub struct Image {
    pub image: PieToken,
    pub offset: u64,
    pub len: u64,
    pub stack: u64,
    pub count: u8,
    #[frame(count = count, fill = 0)]
    pub args: [u64; crate::loader::MAX_ARGS],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Unit(UnitRef),
    Image(Image),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOption(pub Option<Input>);
impl env::wire::Span for InputOption {
    const MAX: Option<usize> = Some(802);
    fn store_at(&self, bytes: &mut [u8], at: usize) -> Option<usize> {
        match &self.0 {
            None => 0u8.store_at(bytes, at),
            Some(input) => {
                let next = 1u8.store_at(bytes, at)?;
                input.store_at(bytes, next)
            }
        }
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (tag, next) = u8::fetch_at(bytes, at)?;
        match tag {
            0 => Some((Self(None), next)),
            1 => Input::fetch_at(bytes, next).map(|(input, end)| (Self(Some(input)), end)),
            _ => None,
        }
    }
}
impl env::wire::Span for Input {
    const MAX: Option<usize> = Some(801);
    fn store_at(&self, bytes: &mut [u8], at: usize) -> Option<usize> {
        match self {
            Self::Unit(value) => {
                let next = 1u8.store_at(bytes, at)?;
                value.store_at(bytes, next)
            }
            Self::Image(value) => {
                let next = 2u8.store_at(bytes, at)?;
                value.store_at(bytes, next)
            }
        }
    }
    fn fetch_at(bytes: &[u8], at: usize) -> Option<(Self, usize)> {
        let (tag, payload) = u8::fetch_at(bytes, at)?;
        let (value, end) = match tag {
            1 => { let (value, end) = UnitRef::fetch_at(bytes, payload)?; (Self::Unit(value), end) }
            2 => { let (value, end) = Image::fetch_at(bytes, payload)?; (Self::Image(value), end) }
            _ => return None,
        };
        Some((value, end))
    }
}
#[derive(env::Frame)]
#[frame(len = 1600)]
pub struct Request {
    pub op: u8,
    pub id: CreateId,
    pub input: InputOption,
    pub owner: TaskId,
    pub subject: Subject,
    pub back: PieToken,
}
impl Message for Request {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (request, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(request)
    }
}
fn reply_to(request: &Request) -> PieToken {
    request.back
}
#[mold::contract(request = Request, response = Said, mark = super::interface::CONSTRUCTION_BACK, back = reply_to)]
pub struct Call;
