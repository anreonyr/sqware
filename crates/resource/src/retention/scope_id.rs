#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeId(usize);

impl ScopeId {
    pub const fn get(self) -> usize {
        self.0
    }

    pub(crate) const fn mint(value: usize) -> Self {
        Self(value)
    }
}
