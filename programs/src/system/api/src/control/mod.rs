//! Pure Control API types and wire contracts.

pub mod account;
pub mod frame;
pub mod grant;
pub mod marks;
pub mod publication;

pub use frame::{ASK_MARK, BACK, DENIED, Fail, Request, Req, Said, State, Wire};
pub use publication::{Frame, Object, Reply, Scope, Target};
pub use grant::{Grant, grant_of};
pub use marks::{DECLARATIONS as MARK_DECLARATIONS, LINK, LINK_MARK};
pub const NAME: &str = frame::NAME;
pub const DIR: &crate::operator::path::Path = frame::DIR;
pub const INSTANCE: &crate::operator::Path = crate::operator::Path::new("/svc/sys/control/instance");
pub const REGISTRY: &[&[env::marks::Definition]] = &[
    &marks::DECLARATIONS,
    &Grant::DECLARATIONS,
];
const _: () = assert!(env::marks::conflict(REGISTRY).is_none());

/// The provider-declared request and response pair.
pub struct Call;
impl wire::Contract for Call {
    type Request = crate::control::Request;
    type Response = crate::control::Said;
}
impl Call {
    pub const BACK: env::Mark = crate::control::BACK;
    pub fn back(request: &(Option<crate::control::Wire>, env::PieToken)) -> env::PieToken { request.1 }
}
