use super::fail;
use crate::system::loader::Loader;
use crate::system::loader::core::source::{Cached, Source};
use env::{Permission, PieToken, TeamId, UnitFail, UnitResult, pie};
use runtime::core::adapt;

impl Drop for Source {
    fn drop(&mut self) {
        if let Some((at, size)) = self.mapping.take() {
            let _ = adapt::munmap(at, size);
        }
        if self.token != PieToken::NONE {
            let _ = pie::release(self.token);
        }
    }
}

pub(super) fn initialize(bytes: &[u8], region: &loader::Region) -> UnitResult<Source> {
    let private = region.flags & 4 != 0;
    let token = if private {
        pie::unseal_pole(region.data_size, false)
    } else {
        pie::unseal_pole(region.data_size, true)
    }
    .map_err(|e| {
        if matches!(e.source, env::PieFail::OoM) {
            fail(UnitFail::OoM)
        } else {
            fail(UnitFail::Denied)
        }
    })?;
    let mut source = Source {
        token,
        mapping: None,
    };
    pie::shut(token).map_err(|_| fail(UnitFail::Denied))?;
    let at = adapt::map(TeamId::new(0), 0, region.data_size, token, 0, 6).map_err(|e| {
        if matches!(e.source, env::MemoryFail::OoM) {
            fail(UnitFail::OoM)
        } else {
            fail(UnitFail::Denied)
        }
    })?;
    source.mapping = Some((at, region.data_size));
    // SAFETY: the parsed ELF payload fits the newly allocated, zeroed writable Pole.
    unsafe {
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr().add(region.file_offset),
            (at + region.prefix) as *mut u8,
            region.file_size,
        )
    };
    if private {
        adapt::munmap(at, region.data_size).map_err(|_| fail(UnitFail::Denied))?;
        source.mapping = None;
    }
    Ok(source)
}

impl Loader {
    pub(super) fn shared(&mut self, bytes: &[u8], region: &loader::Region) -> UnitResult<PieToken> {
        if let Some(item) = self.cache.iter().find(|item| {
            item.va == region.va && item.flags == region.flags && item.source.matches(bytes, region)
        }) {
            return Ok(item.source.token);
        }
        let source = initialize(bytes, region)?;
        let token = source.token;
        pie::narrow(token, Permission::FETCH | Permission::VEST)
            .map_err(|_| fail(UnitFail::Denied))?;
        self.cache.try_reserve(1).map_err(|_| fail(UnitFail::OoM))?;
        self.cache.push(Cached {
            va: region.va,
            flags: region.flags,
            source,
        });
        Ok(token)
    }
}
