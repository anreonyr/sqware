#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainId(u64);

impl DomainId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}
