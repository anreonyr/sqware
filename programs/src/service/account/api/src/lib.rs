#![no_std]
extern crate alloc;
pub mod frame;
pub use frame::Request;
pub use interface::{BACK, CHANNELS, ENTRY, INTERFACE_ID, REGISTRY};
pub const DIR: &system_api::operator::Path = system_api::operator::Path::new("/svc/account/create");
fn reply_to(request: &(Request, bool)) -> env::PieToken {
    request.0.back
}
#[mold::contract(request = Request, response = system_api::loader::Said, mark = BACK, back = reply_to)]
pub struct Call;

#[mold::interface(id = "sqware.service.account.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "create", legacy = "control-account", constant = ENTRY, publication = "create")]
        Create,
        #[channel(key = "reply", legacy = "control-account-back", constant = BACK)]
        Reply,
    }
}
