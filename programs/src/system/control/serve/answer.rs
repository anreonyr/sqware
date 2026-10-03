use super::{lifecycle::{Action, Operation, Operations, Request}, unit::Control};
use crate::system::control::core::unit::State;
use env::{PieToken, Wait};
use protocol::{communication::hand::Sender, system::control as ccall};
use runtime::env::mail::{self, HolePie};

pub(crate) fn serve_face(control: &Control, operations: &mut Operations,
    grant: ccall::Grant, face: PieToken, buf: &mut [u8]) {
    let hole = HolePie::from_token(face);
    while let Ok((len, from)) = hole.pull(buf, Wait::POLL) {
        let Some((ask, back)) = ccall::frame::Wire::take(&buf[..len]) else { continue; };
        if !matches!(mail::reserve(back), Ok((_, owner, mark)) if owner == from && mark == ccall::BACK) { continue; }
        let Some(ask) = ask else { reply(back, ccall::frame::said_status(ccall::frame::BAD)); continue; };
        if ccall::Grant::for_wire(&ask) != grant { reply(back, ccall::frame::said_status(ccall::frame::DENIED)); continue; }
        let (name, action) = match ask {
            ccall::frame::Wire::State(name) => {
                let said = match control.state(name) {
                    Ok(state) => ccall::frame::said_state(wire_state(state)),
                    Err(fail) => status(fail),
                };
                reply(back, said); continue;
            }
            ccall::frame::Wire::Mint(name) => (name, Action::Mint),
            ccall::frame::Wire::Embark(name) => (name, Action::Embark { parent: Some(from) }),
            ccall::frame::Wire::Debark(name) => (name, Action::Debark),
            ccall::frame::Wire::Ruin(name) => (name, Action::Ruin),
        };
        if let Err(fail) = operations.push(Request { name, action, back: Some(back) }) { reply(back, status(fail)); }
    }
}
pub(crate) fn complete(operation: &Operation) {
    let Some(back) = operation.request.back else { return; };
    let said = match operation.failure {
        Some(fail) => status(fail),
        None => match operation.request.action {
            Action::Embark { .. } => match operation.task {
                Some(task) => ccall::frame::said_task(task),
                None => status(crate::system::control::core::verdict::Fail::NotReady),
            },
            _ => ccall::frame::said_status(ccall::frame::OK),
        },
    };
    reply(back, said);
}
fn status(fail: crate::system::control::core::verdict::Fail) -> ccall::frame::Said {
    ccall::frame::said_status(ccall::frame::fail_to_code(Some(wire_fail(fail))))
}
fn reply(back: PieToken, said: ccall::frame::Said) {
    { let mut tx = Sender::<ccall::frame::Said>::from_token(back); let _ = tx.send(said); }
    let _ = mail::release(back);
}
fn wire_fail(fail: crate::system::control::core::verdict::Fail) -> ccall::Fail {
    use crate::system::control::core::verdict::Fail as Model;
    match fail {
        Model::Unknown => ccall::Fail::Unknown,
        Model::BadImage => ccall::Fail::BadImage,
        Model::Full => ccall::Fail::Full,
        Model::NotReady => ccall::Fail::NotReady,
    }
}
fn wire_state(state: State) -> ccall::State {
    match state {
        State::NeverStarted => ccall::State::NeverStarted,
        State::Starting => ccall::State::Starting,
        State::Ready => ccall::State::Ready,
        State::Stopping => ccall::State::Stopping,
        State::Dead => ccall::State::Dead,
        State::Debarked => ccall::State::Debarked,
    }
}
