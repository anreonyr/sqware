pub struct Accounts {
    view: &'static [u8],
}

impl Accounts {
    pub fn new(view: &'static [u8]) -> Self {
        Self { view }
    }

    pub fn view(&self) -> &'static [u8] {
        self.view
    }
}
