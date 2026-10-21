pub struct References;

impl References {
    pub fn len(&self) -> usize {
        crate::REFERENCES.with(|count| count.get())
    }
}
