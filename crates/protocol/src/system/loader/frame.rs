use crate::common::path::Path;
pub use crate::system::control::Fail;
use crate::wire::message::Message;
use env::wire::Span;
use env::{Mark, PieToken, TaskId};
pub const DIR: &Path = Path::new("svc/sys/loader");
pub const BACK: Mark = Mark::of("loader-back");
pub const IMAGE: Mark = Mark::of("loader-image");
pub const BUILD: u8 = 1;
pub const CLAIM: u8 = 2;
pub const CLAIM_MS: usize = 3000;
pub const MAX_ARGS: usize = 64;
pub const MAX_IMAGE: usize = 16 * 1024 * 1024;

#[derive(env::Frame)]
pub struct Ask {
    pub op: u8,
    pub image: PieToken,
    pub offset: u64,
    pub len: u64,
    pub stack: u64,
    pub count: u8,
    #[frame(count = count, fill = 0)]
    pub args: [u64; MAX_ARGS],
    pub back: PieToken,
}
#[derive(env::Frame)]
pub struct Claim {
    pub op: u8,
    pub task: TaskId,
    pub back: PieToken,
}
pub enum Wire {
    Build(Ask),
    Claim(Claim),
}
impl Wire {
    pub fn take(bytes: &[u8]) -> Option<Self> {
        if bytes.first() == Some(&CLAIM) {
            let (claim, end) = Claim::fetch_at(bytes, 0)?;
            return (end == bytes.len()).then_some(Self::Claim(claim));
        }
        let (ask, end) = Ask::fetch_at(bytes, 0)?;
        if ask.op != BUILD {
            return None;
        }
        (end == bytes.len()).then_some(Self::Build(ask))
    }
}
#[derive(env::Frame, Clone, Copy, Debug)]
pub struct Said {
    pub status: u8,
    pub team: u64,
    pub task: TaskId,
}
impl Message for Said {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = Self::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}
