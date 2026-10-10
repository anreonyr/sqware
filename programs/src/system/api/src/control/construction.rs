pub use super::marks::{CONSTRUCTION_BACK as BACK, CONSTRUCTION_ENTRY as ENTRY};
use crate::{
    identity::Subject,
    loader::{Ask, Said},
};
use env::wire::Span as _;
use env::{PieToken, TaskId};
use wire::Message;
#[derive(env::Frame)]
#[frame(len = 1024)]
pub struct Request {
    pub image: Ask,
    pub owner: TaskId,
    pub subject: Subject,
    pub constructor: bool,
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
    request.image.back
}
#[mold::contract(request = Request, response = Said, mark = super::interface::CONSTRUCTION_BACK, back = reply_to)]
pub struct Call;
