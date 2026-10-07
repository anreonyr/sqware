//! Pure identity API vocabulary, bounds, marks, and wire representation.

pub mod frame;
pub mod grant;
pub mod limits;
pub mod marks;

pub use frame::vocab::*;
pub use frame::{BACK, Fail, Reply, Request, Wire};
pub use grant::{Grant, Mount, grant_of};
pub const DIR: &str = frame::DIR;
pub const REGISTRY: &[&[env::marks::Definition]] = &[&marks::DECLARATIONS, &Grant::DECLARATIONS];

const _: () = assert!(env::marks::conflict(REGISTRY).is_none(), "identity mark collision");
