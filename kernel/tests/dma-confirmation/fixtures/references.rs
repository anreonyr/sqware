pub struct References;

impl References {
    pub fn len(&self) -> usize {
        crate::REFERENCES.with(|count| count.get())
    }

    pub fn get(&self, reference: usize) -> Option<crate::hold::Hold> {
        (reference == 777 && self.len() == 2).then_some(crate::hold::Hold)
    }
}
