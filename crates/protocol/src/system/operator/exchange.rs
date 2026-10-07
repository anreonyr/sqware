//! Operator request/reply binding for an established session.

pub struct Contract;

impl ipc::session::Contract for Contract {
    type Request = super::Req;
    type Response = super::Union;
}
