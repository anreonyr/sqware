use super::image::elf;
use env::PieToken;

/// 一块已装好的来源副本：地址 / 长度 / **准确的关系身份**（归还按它执行）。
pub(in crate::system::loader) struct Source {
    pub token: PieToken,
    pub mapping: Option<execution::memory::View>,
}

impl Source {
    pub(in crate::system::loader) fn matches(&self, bytes: &[u8], region: &elf::Region) -> bool {
        let Some(mapping) = &self.mapping else {
            return false;
        };
        let (at, size) = (mapping.address(), mapping.size());
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
