//! Loader request/reply binding for the IPC transport.

use env::{Mark, PieToken};
use super::frame::{self, Said, Wire};

pub struct Contract;

impl ipc::rpc::Contract for Contract {
    type Request = Wire;
    type Response = Said;
    const BACK: Mark = frame::BACK;

    fn back(request: &Wire) -> PieToken {
        match request {
            Wire::Build(ask) => ask.back,
            Wire::Claim(claim) => claim.back,
        }
    }
}
