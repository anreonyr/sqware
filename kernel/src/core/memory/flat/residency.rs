use super::FlatRange;

/// Blocks ownership changes for this logical window until the last actual holder drops it.
pub struct Residency {
    range: FlatRange,
    id: usize,
}

impl Residency {
    pub(super) fn new(range: FlatRange, id: usize) -> Self {
        Self { range, id }
    }

    pub fn range(&self) -> &FlatRange {
        &self.range
    }
}

impl Drop for Residency {
    fn drop(&mut self) {
        self.range.release_residency(self.id);
    }
}
