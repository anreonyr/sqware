use crate::machine::hart::HartId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Owner(usize);

impl Owner {
    pub(super) const FREE: Owner = Owner(0);

    pub(super) fn of(hart: HartId) -> Owner {
        Owner(hart.get() + 1)
    }

    pub(super) fn word(self) -> usize {
        self.0
    }

    pub(super) fn from_word(w: usize) -> Owner {
        Owner(w)
    }
}
