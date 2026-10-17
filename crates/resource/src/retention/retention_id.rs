#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionId(usize);

impl RetentionId {
    pub(crate) const fn mint(value: usize) -> Self {
        Self(value)
    }
}
