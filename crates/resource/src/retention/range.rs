#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressRange {
    base: u64,
    length: u64,
}

impl AddressRange {
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

    pub(super) fn overlaps(self, other: Self) -> bool {
        self.base <= other.base + other.length - 1 && other.base <= self.base + self.length - 1
    }
}
