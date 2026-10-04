//! Identity owns the identity book and its request faces.
#[derive(Debug)]
pub enum Fail {
    Book,
    Room,
    Tree,
    Desk,
    Dead,
}
mod answer;
mod book;
mod face;
mod frame;
pub mod install;
pub mod names;
pub mod query;
pub mod revision;
pub mod run;
mod schedule;
pub mod source;

pub mod publication;
