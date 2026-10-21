pub struct Retirement {
    pub state: u8,
}

impl Retirement {
    pub fn settled(&self) -> bool {
        self.state == 2
    }

    pub fn done() -> Self {
        Self { state: 2 }
    }
}
