#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Cell {
    owner: Option<u8>,
    pub(super) power: u8,
    used: u16,
}

impl Cell {
    pub(super) const fn free() -> Self {
        Self {
            owner: None,
            power: 0,
            used: 0,
        }
    }

    pub(super) const fn new(owner: usize, power: usize) -> Self {
        Self {
            owner: Some(owner as u8),
            power: power as u8,
            used: 1,
        }
    }

    pub(super) fn owner(self) -> Option<usize> {
        self.owner.map(|o| o as usize)
    }

    pub(super) fn used(self) -> u16 {
        self.used
    }

    pub(super) fn inc_used(self) -> Self {
        Self {
            used: self.used.saturating_add(1),
            ..self
        }
    }

    pub(super) fn dec_used(self) -> (Self, bool) {
        let used = self.used.saturating_sub(1);
        (Self { used, ..self }, used == 0)
    }
}
