//! Identity owns the identity book and its request faces.
#[derive(Debug)]
pub enum Fail { Book, Room, Tree, Desk, Dead }
mod answer;
mod book;
mod face;
mod frame;
mod schedule;
pub mod run;
pub mod install;
pub mod source;
pub mod query;
pub mod names;
pub mod revision;
