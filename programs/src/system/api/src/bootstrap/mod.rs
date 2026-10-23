//! Generic bootstrap frame shared by Assembly and constructed children.

pub mod fail;
pub mod frame;

pub use fail::Fail;
pub use frame::Frame;

pub use crate::loader::BOOTSTRAP as MARK;
pub const VERSION: u8 = 1;
pub const ROLE: u8 = 1;
