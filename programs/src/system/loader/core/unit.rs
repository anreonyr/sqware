use super::cache::Cache;
use alloc::vec::Vec;
use env::{PieToken, ProgramKind, TeamId};

pub struct Image<'a> {
    pub bytes: &'a [u8],
    pub kind: ProgramKind,
}

const CACHE_PAGES: usize = 256;

pub struct Loader {
    pub(in crate::system::loader) cache: Cache,
}
impl Loader {
    pub fn clear(&mut self) {
        self.cache.clear();
    }
    pub fn new() -> Self {
        Self {
            cache: Cache::new(CACHE_PAGES),
        }
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
