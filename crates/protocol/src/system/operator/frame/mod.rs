//! Compatibility reexports for the Operator wire API.

pub mod road { pub use system_api::operator::frame::road::*; }
pub mod tip { pub use system_api::operator::frame::tip::*; }
pub mod vocab { pub use system_api::operator::frame::vocab::*; }
pub mod watch { pub use system_api::operator::frame::watch::*; }

pub use system_api::operator::frame::*;

impl crate::wire::id::Id for EntryId {
    fn new(raw: usize) -> Self { Self::new(raw) }
    fn get(self) -> usize { Self::get(self) }
}
