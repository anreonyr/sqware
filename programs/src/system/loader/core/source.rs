use env::PieToken;

pub(in crate::system::loader) struct Cached {
    pub flags: u64,
    pub source: Source,
}

pub(in crate::system::loader) struct Source {
    pub token: PieToken,
    pub mapping: Option<(usize, usize)>,
}

impl Source {
    pub(in crate::system::loader) fn matches(&self, bytes: &[u8], region: &loader::Region) -> bool {
        let Some((at, size)) = self.mapping else {
            return false;
        };
        if size != region.data_size {
            return false;
        }
        // SAFETY: cached sources retain a read-only mapping until the source is dropped.
        let mapped = unsafe { core::slice::from_raw_parts(at as *const u8, size) };
        let end = region.prefix + region.file_size;
        mapped[..region.prefix].iter().all(|&byte| byte == 0)
            && mapped[region.prefix..end]
                == bytes[region.file_offset..region.file_offset + region.file_size]
            && mapped[end..].iter().all(|&byte| byte == 0)
    }
}
