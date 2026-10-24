use super::{Said, Wire, BACK};

fn back(wire: &Wire) -> env::PieToken {
    match wire {
        Wire::Observe(frame) => frame.back,
        Wire::Supply(frame) => frame.back,
    }
}

#[mold::contract(request = Wire, response = Said, mark = BACK, back = back)]
pub struct Call;
