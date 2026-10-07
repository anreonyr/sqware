use crate::system::loader::core::elf;
use super::source::{Cached, Source};
use alloc::vec::Vec;
use env::PieToken;

pub(in crate::system::loader) struct Cache {
    entries: Vec<Cached>,
    pages: usize,
    limit: usize,
}
impl Cache {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            pages: 0,
            limit,
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.pages = 0;
    }
    pub fn find(&self, bytes: &[u8], region: &elf::Region) -> Option<PieToken> {
        self.entries
            .iter()
            .find(|item| item.flags == region.flags && item.source.matches(bytes, region))
            .map(|item| item.source.token)
    }
    pub fn insert(&mut self, region: &elf::Region, source: Source) -> Result<(), Source> {
        let pages = region.data_size / env::PAGE_SIZE;
        if pages > self.limit || self.entries.try_reserve(1).is_err() {
            return Err(source);
        }
        while self.pages + pages > self.limit {
            let old = self.entries.remove(0);
            self.pages -= old.source.mapping.unwrap().1 / env::PAGE_SIZE;
        }
        self.pages += pages;
        self.entries.push(Cached {
            flags: region.flags,
            source,
        });
        Ok(())
    }
}
