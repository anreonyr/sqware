pub struct Settle {
    pub settled: bool,
}

impl Settle {
    pub fn gate(&mut self, _: &crate::unit::UnitFile) -> bool {
        self.settled
    }
}
