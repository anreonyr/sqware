pub mod frame;
pub use super::marks::{ACCOUNT_BACK as BACK, ACCOUNT_ENTRY as ENTRY};
pub use frame::Request;
pub const DIR: &crate::operator::Path = crate::operator::Path::new("/svc/sys/control/account");

fn reply_to(request: &(Request, bool)) -> env::PieToken {
    request.0.back
}

#[mold::contract(
    request = Request,
    response = crate::loader::Said,
    mark = super::interface::ACCOUNT_BACK,
    back = reply_to
)]
pub struct Call;
