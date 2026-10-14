use super::hand::Hand;

pub(super) enum Slot {
    Reserved(usize, usize),
    Ready(Hand),
}

impl Slot {
    pub(super) fn hand(&self) -> Option<&Hand> {
        match self {
            Self::Ready(hand) => Some(hand),
            Self::Reserved(..) => None,
        }
    }
}
