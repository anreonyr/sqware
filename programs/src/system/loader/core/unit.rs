use super::source::Cached;
use alloc::vec::Vec;
use env::{PieToken, ProgramKind, TeamId};

pub struct Image<'a> {
    pub bytes: &'a [u8],
    pub kind: ProgramKind,
}

pub struct Loader {
    pub(in crate::system::loader) cache: Vec<Cached>,
}
impl Loader {
    pub fn new() -> Self {
        Self { cache: Vec::new() }
    }
}

pub struct Unit {
    pub(in crate::system::loader) team: TeamId,
    pub(in crate::system::loader) entry: usize,
    pub(in crate::system::loader) private: Vec<PieToken>,
    pub(in crate::system::loader) committed: bool,
}
impl Unit {
    pub fn team(&self) -> TeamId {
        self.team
    }
}
