use crate::unit::{Died, hub::E_HUB};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Tree(Died),
    Room(Died),
    Desk(Died),
    Book(Died),
    Face(Died),
    Load(Died),
    Dead(Died),
}

impl Start {
    pub fn code(self) -> env::Reason {
        match self {
            Start::Tree(d)
            | Start::Room(d)
            | Start::Desk(d)
            | Start::Book(d)
            | Start::Face(d)
            | Start::Load(d)
            | Start::Dead(d) => d,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Start::Tree(E_HUB) => "hub: tree",
            Start::Load(E_HUB) => "hub: no machine",
            Start::Face(E_HUB) => "hub: no league plate",
            Start::Room(E_HUB) => "hub: no room",
            Start::Desk(E_HUB) => "hub: desk",
            Start::Dead(E_HUB) => "inner: group dead",
            _ => "start: ?",
        }
    }
}

impl crate::Exit for Start {
    fn report(&self) -> crate::Report<'_> {
        crate::Report::note(self.code(), self.text())
    }
}
