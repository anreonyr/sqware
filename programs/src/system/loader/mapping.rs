use super::fail;
use super::image::elf;
use super::source::Source;
use env::{PieToken, TeamId, UnitFail, UnitResult, pie};

impl Drop for Source {
    fn drop(&mut self) {
        if let Some(view) = &mut self.mapping {
            let _ = view.close();
            view.release_source()
                .expect("loader source reference cleanup");
        }
        if self.token != PieToken::NONE {
            let _ = pie::release(self.token, env::ReleaseMode::Revoke);
        }
    }
}

pub(super) fn initialize(bytes: &[u8], region: &elf::Region) -> UnitResult<Source> {
    let private = region.flags & 4 != 0;
    let token = ::resource::raw::pole(region.data_size, !private).map_err(|e| {
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
    let view = execution::memory::map(
        TeamId::new(0),
        0,
        region.data_size,
        token,
        0,
        6,
        env::ViewControl::Caller,
    )
    .map_err(|e| {
        if matches!(e.source, env::MemoryFail::OoM) {
            fail(UnitFail::OoM)
        } else {
            fail(UnitFail::Denied)
        }
    })?;
    let at = view.address();
    source.mapping = Some(view);
    // SAFETY: the parsed ELF payload fits the newly allocated, zeroed writable Pole.
    unsafe {
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr().add(region.file_offset),
            (at + region.prefix) as *mut u8,
            region.file_size,
        )
    };
    if private {
        source
            .mapping
            .as_mut()
            .unwrap()
            .close()
            .map_err(|_| fail(UnitFail::Denied))?;
        source.mapping = None;
    }
    Ok(source)
}
