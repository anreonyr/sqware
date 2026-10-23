use system_api::control::{State, observe::Reply};

/// What one successful observation asks the caller to do next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    Recover,
    Advance,
    End,
}

pub fn decide(reply: &Reply) -> Decision {
    if reply.end {
        return Decision::End;
    }
    if reply.present
        && State::of_code(reply.state) == Some(State::Dead)
        && reply.run_requested
    {
        return Decision::Recover;
    }
    Decision::Advance
}
