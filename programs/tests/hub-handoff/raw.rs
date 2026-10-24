use env::{Mark, PieKind, PieToken};

mod hole;
pub use hole::Hole;

pub struct Info {
    pub alive: bool,
    pub kind: PieKind,
    pub mark: Mark,
    pub token: PieToken,
}

pub fn pies() -> impl Iterator<Item = Info> {
    ::core::iter::empty()
}
