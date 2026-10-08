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
mod revision;
pub mod run;
mod schedule;
