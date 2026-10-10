mod build;
mod cache;
mod image;
mod mapping;
mod service;
mod source;
mod task;

pub(crate) use build::{Source, snapshot};
pub(crate) use image::Image;
pub(crate) use service::release_image;
pub(crate) use service::{entry, faces, frame, install, shutdown, watch};
pub(crate) use task::Unit;

use cache::Cache;
use env::UnitResult;

const CACHE_PAGES: usize = 256;

pub(crate) struct Loader {
    cache: Cache,
}

pub(crate) struct Spawn<'a> {
    pub args: &'a [usize],
    pub stack: usize,
}

pub(super) fn fail(source: env::UnitFail) -> erra::Error<env::UnitFail> {
    erra::Error::new("constructing ELF image", source)
}

impl Loader {
    pub(crate) fn new() -> Self {
        Self {
            cache: Cache::new(CACHE_PAGES),
        }
    }

    pub(crate) fn clear_images(&mut self) {
        self.cache.clear();
    }

    pub(crate) fn cached_entries(&self) -> usize {
        self.cache.entry_count()
    }

    pub(crate) fn build(&mut self, image: Image<'_>) -> UnitResult<Unit> {
        build::load(self, image)
    }

    pub(crate) fn construct(
        &mut self,
        image: Image<'_>,
        spawn: Spawn<'_>,
    ) -> Result<system_api::loader::Built, system_api::loader::Fail> {
        build::construct(self, image, spawn)
    }
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) use service::Inbox;
