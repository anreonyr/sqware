use alloc::string::String;
use ipc::rpc::reply::Sender as ReplySender;
use system_api::control::frame::Said;

use crate::system::control::lifecycle::Directive;

pub struct Request {
    pub name: String,
    pub directive: Directive,
    pub back: Option<ReplySender<Said>>,
}
