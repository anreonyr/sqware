pub mod frame;
pub use frame::Request;
pub use super::marks::{ACCOUNT_ENTRY as ENTRY, ACCOUNT_BACK as BACK};
pub const DIR: &crate::operator::Path = crate::operator::Path::new("/svc/sys/control/account");
