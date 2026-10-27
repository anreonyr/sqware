//! 这一面的契约：一问一答、回信孔那一格随动作码走。

use env::PieToken;

use super::{BACK, Said, Wire};

fn reply_to(wire: &Wire) -> PieToken {
    match wire {
        Wire::Establish(frame) => frame.back,
        Wire::Acquire(frame) => frame.back,
        Wire::Publish(frame) => frame.back,
        Wire::Invalidate(frame) => frame.back,
        Wire::Quiesce(frame) => frame.back,
        Wire::Release(frame) => frame.back,
    }
}

#[mold::contract(request = Wire, response = Said, mark = BACK, back = reply_to)]
pub struct Call;
