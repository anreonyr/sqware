use crate::system::loader::core::elf;
use super::fail;
use crate::system::loader::core::source::Source;
use env::{PieToken, TeamId, UnitFail, UnitResult, pie};

impl Drop for Source {
    fn drop(&mut self) {
        if let Some((at, size)) = self.mapping.take() {
            let _ = runtime::core::memory::munmap(at, size);
        }
        if self.token != PieToken::NONE {
            let _ = pie::release(self.token);
        }
    }
}

pub(super) fn initialize(bytes: &[u8], region: &elf::Region) -> UnitResult<Source> {
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
    let at = runtime::core::memory::map(TeamId::new(0), 0, region.data_size, token, 0, 6).map_err(|e| {
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
        runtime::core::memory::munmap(at, region.data_size).map_err(|_| fail(UnitFail::Denied))?;
        source.mapping = None;
    }
    Ok(source)
}
