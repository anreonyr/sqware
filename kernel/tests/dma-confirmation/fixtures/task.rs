pub struct Task {
    pub pend: crate::pend::Pend,
}

impl Task {
    pub fn upgrade(self) -> Option<Self> {
        Some(self)
    }
}
