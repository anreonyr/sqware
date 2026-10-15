use super::source::Source;

pub(in crate::system::loader) struct Cached {
    pub flags: u64,
    pub source: Source,
}
