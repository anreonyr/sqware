use super::FlatRange;

pub struct Residency {
    range: FlatRange,
}

impl Residency {
    pub fn new(range: FlatRange) -> Self {
        Self { range }
    }

    pub fn range(&self) -> &FlatRange {
        &self.range
    }
}
