pub mod frame;
pub use frame::Request;
pub use super::marks::{ACCOUNT_ENTRY as ENTRY, ACCOUNT_BACK as BACK};
pub const DIR: &crate::operator::Path = crate::operator::Path::new("/svc/sys/control/account");

/// The provider-declared request and response pair.
pub struct Call;
impl wire::Contract for Call {
    type Request = crate::control::account::Request;
    type Response = crate::loader::Said;
}
impl Call {
    pub const BACK: env::Mark = crate::control::account::BACK;
    pub fn back(request: &(crate::control::account::Request, bool)) -> env::PieToken { request.0.back }
}
