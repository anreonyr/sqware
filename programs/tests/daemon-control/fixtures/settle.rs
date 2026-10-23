pub struct Settle {
    pub closing: bool,
}

impl Settle {
    pub fn closing(&self) -> bool {
        self.closing
    }
}
