//! Strict, variable-length framing; no allocation or ignored trailing bytes.
mod codec;
mod data;
pub mod vocab;

pub use super::limits::MAX_FRAME;
pub use crate::wire::OK;
pub use vocab::{Fail, Reply, Wire, code_to_fail, fail_to_code};

pub const BACK: env::Mark = env::Mark::of("identity-back");
pub const NAME: &str = "identity";
pub const DIR: &str = "/svc/sys/identity";
