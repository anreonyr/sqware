//! Strict, variable-length identity framing; no allocation or ignored trailing bytes.
mod codec;
pub use codec::Request;
mod data;
pub mod vocab;

pub use super::limits::MAX_FRAME;
pub use super::marks::BACK;
pub use vocab::{Fail, Reply, Wire, code_to_fail, fail_to_code};
pub use wire::OK;
pub const NAME: &str = "identity";
pub const DIR: &str = "/svc/sys/identity";
