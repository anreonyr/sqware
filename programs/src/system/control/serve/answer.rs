use super::{
    lifecycle::{Action, Operation, Operations, Request},
    unit::Control,
};
use crate::system::control::core::unit::State;
use env::pie;
use env::{PieToken, Wait};
use ipc::hand::Sender;
use protocol::system::control as ccall;
use ::resource::raw::{Hole, reserve};

pub struct Incoming {
    pub wire: ccall::frame::Wire,
    pub from: env::TaskId,
    pub back: PieToken,
}
pub struct Inbox(pub alloc::collections::VecDeque<Incoming>);
pub struct Buffer(pub alloc::vec::Vec<u8>);

pub fn receive(
    watch: ::schedule::Res<super::watch::Watch>,
    mut buffer: ::schedule::ResMut<Buffer>,
    mut inbox: ::schedule::ResMut<Inbox>,
) -> Result<::schedule::Progress, super::Fail> {
    for grant in ccall::Grant::ALL {
        let Some(face) = watch.faces[grant.index()] else {
            continue;
        };
        let hole = Hole::from_raw(face);
        while let Ok((len, from)) = hole.pull(&mut buffer.0, Wait::POLL) {
            let Some((ask, back)) = ccall::frame::Wire::take(&buffer.0[..len]) else {
                continue;
            };
            if !matches!(reserve(back), Ok((_, owner, mark)) if owner == from && mark == ccall::BACK)
            {
                continue;
            }
            let Some(wire) = ask else {
                reply(back, ccall::frame::said_status(ccall::frame::BAD));
                continue;
            };
            if ccall::Grant::for_wire(&wire) != grant {
                reply(back, ccall::frame::said_status(ccall::frame::DENIED));
                continue;
            }
            if inbox.0.try_reserve(1).is_err() {
                reply(
                    back,
                    status(crate::system::control::core::verdict::Fail::Full),
                );
                continue;
            }
            inbox.0.push_back(Incoming { wire, from, back });
        }
    }
    Ok(::schedule::Progress::Done)
}
pub fn state(
    control: ::schedule::Res<Control>,
    mut inbox: ::schedule::ResMut<Inbox>,
) -> Result<::schedule::Progress, super::Fail> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(super::Fail::Room)?;
        if let ccall::frame::Wire::State(name) = incoming.wire {
            let said = match control.state(name) {
                Ok(state) => ccall::frame::said_state(wire_state(state)),
                Err(fail) => status(fail),
            };
            reply(incoming.back, said);
        } else {
            inbox.0.push_back(incoming);
        }
    }
    Ok(::schedule::Progress::Done)
}
pub fn enqueue(
    mut operations: ::schedule::ResMut<Operations>,
    mut inbox: ::schedule::ResMut<Inbox>,
) -> Result<::schedule::Progress, super::Fail> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(super::Fail::Room)?;
        let (name, action) = match incoming.wire {
            ccall::frame::Wire::Mint(name) => (name, Action::Mint),
            ccall::frame::Wire::Embark(name) => (
                name,
                Action::Embark {
                    parent: Some(incoming.from),
                },
            ),
            ccall::frame::Wire::Debark(name) => (name, Action::Debark),
            ccall::frame::Wire::Ruin(name) => (name, Action::Ruin),
            ccall::frame::Wire::State(_) => return Err(super::Fail::Room),
            _ => {
                inbox.0.push_back(incoming);
                continue;
            }
        };
        if let Err(fail) = operations.push(Request {
            name,
            action,
            back: Some(incoming.back),
        }) {
            reply(incoming.back, status(fail));
        }
    }
    Ok(::schedule::Progress::Done)
}
pub(crate) fn complete(operation: &Operation) {
    let Some(back) = operation.request.back else {
        return;
    };
    let said = match operation.failure {
        Some(fail) => status(fail),
        None => match operation.request.action {
            Action::Embark { .. } => match operation.execution.task {
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
pub(super) fn reply(back: PieToken, said: ccall::frame::Said) {
    {
        let mut tx = Sender::<ccall::frame::Said>::from_raw(back);
        let _ = tx.send(said);
    }
    let _ = pie::release(back);
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
pub(super) fn wire_state(state: State) -> ccall::State {
    match state {
        State::NeverStarted => ccall::State::NeverStarted,
        State::Starting => ccall::State::Starting,
        State::Ready => ccall::State::Ready,
        State::Stopping => ccall::State::Stopping,
        State::Dead => ccall::State::Dead,
        State::Debarked => ccall::State::Debarked,
    }
}
