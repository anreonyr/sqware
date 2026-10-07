//! Identity request/reply binding for the IPC transport.

use super::{BACK, Reply, Wire, frame::Request};
use env::{Mark, PieToken};

pub struct Contract;

impl ipc::rpc::Contract for Contract {
    type Request = Request;
    type Response = Reply;
    const BACK: Mark = BACK;

    fn back(request: &(Option<Wire>, PieToken)) -> PieToken {
        request.1
    }
}
