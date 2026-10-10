//! Identity owns the identity book and its request faces.
#[derive(Debug)]
pub enum Fail {
    Book,
    Room,
    Tree,
    Desk,
    Dead,
}
impl From<::schedule::resource::AccessError> for Fail {
    fn from(_: ::schedule::resource::AccessError) -> Self {
        Self::Room
    }
}
impl From<::schedule::BuildError> for Fail {
    fn from(_: ::schedule::BuildError) -> Self {
        Self::Room
    }
}
impl From<::schedule::DispatchError> for Fail {
    fn from(_: ::schedule::DispatchError) -> Self {
        Self::Room
    }
}
impl From<alloc::collections::TryReserveError> for Fail {
    fn from(_: alloc::collections::TryReserveError) -> Self {
        Self::Room
    }
}
impl From<erra::Error<env::PieFail>> for Fail {
    fn from(_: erra::Error<env::PieFail>) -> Self {
        Self::Desk
    }
}

mod answer;
mod book;
mod face;
mod frame;
mod revision;
pub mod run;
mod schedule;
