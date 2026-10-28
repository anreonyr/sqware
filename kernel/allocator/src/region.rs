#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub base: usize,
    pub size: usize,
}

impl Region {
    pub const fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }
}
