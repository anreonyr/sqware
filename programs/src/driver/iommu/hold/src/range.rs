#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceRange {
    base: u64,
    length: u64,
}

impl DeviceRange {
    pub const fn new(base: u64, length: u64) -> Option<Self> {
        if length == 0 || base.checked_add(length - 1).is_none() {
            return None;
        }
        Some(Self { base, length })
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn length(self) -> u64 {
        self.length
    }

    /// 闭区间末地址；构造器已保证 `base + length - 1` 不溢出。
    const fn end(self) -> u64 {
        self.base + (self.length - 1)
    }

    pub(crate) fn overlaps(self, other: Self) -> bool {
        self.base <= other.end() && other.base <= self.end()
    }
}
