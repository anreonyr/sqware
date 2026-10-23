use crate::system::control::lifecycle::{Action, Directive, Origin, Request};
use crate::system::control::unit::Control;
use crate::system::control::unit::table::State;
use env::Wait;
use ipc::rpc::{self, reply::Sender as ReplySender, request::Receiver as RequestReceiver};
use system_api::control as ccall;
use system_api::control::Call as ControlContract;

pub struct Incoming {
    pub wire: ccall::frame::Wire,
    pub from: env::TaskId,
    pub reply: ReplySender<ccall::frame::Said>,
}
pub struct Inbox(pub alloc::collections::VecDeque<Incoming>);
pub struct Buffer(pub alloc::vec::Vec<u8>);

pub fn receive(
    watch: ::schedule::Res<crate::system::control::namespace::Endpoints>,
    mut buffer: ::schedule::ResMut<Buffer>,
    mut inbox: ::schedule::ResMut<Inbox>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    for grant in ccall::Grant::ALL {
        if grant == ccall::Grant::Observe {
            continue;
        }
        let Some(face) = watch.face(grant) else {
            continue;
        };
        let receiver = RequestReceiver::<ControlContract>::from_raw(
            face,
            ControlContract::BACK,
            ControlContract::back,
        );
        loop {
            let request = match receiver.receive(&mut buffer.0, Wait::POLL) {
                Ok(request) => request,
                Err(rejected) => match rejected.fail {
                    rpc::Fail::Receive(_) => break,
                    _ => continue,
                },
            };
            let from = request.from;
            let (ask, _) = request.request;
            let reply_tx = request.reply;
            let Some(wire) = ask else {
                reply(reply_tx, ccall::frame::said_status(ccall::frame::BAD));
                continue;
            };
            if ccall::Grant::for_wire(&wire) != grant {
                reply(reply_tx, ccall::frame::said_status(ccall::frame::DENIED));
                continue;
            }
            if inbox.0.try_reserve(1).is_err() {
                reply(
                    reply_tx,
                    status(crate::system::control::unit::verdict::Fail::Full),
                );
                continue;
            }
            inbox.0.push_back(Incoming {
                wire,
                from,
                reply: reply_tx,
            });
        }
    }
    Ok(::schedule::Progress::Done)
}
pub fn state(
    control: ::schedule::Res<Control>,
    mut inbox: ::schedule::ResMut<Inbox>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(crate::system::app::Fault::Room)?;
        let said = match incoming.wire {
            ccall::frame::Wire::State(name) => match control.state(name) {
                Ok(state) => ccall::frame::said_state(wire_state(state)),
                Err(fail) => status(fail),
            },
            ccall::frame::Wire::Task(name) => match control.named_task(&name) {
                Ok(task) => ccall::frame::said_task(task),
                Err(fail) => status(fail),
            },
            _ => {
                inbox.0.push_back(incoming);
                continue;
            }
        };
        reply(incoming.reply, said);
    }
    Ok(::schedule::Progress::Done)
}
pub fn enqueue(
    mut operations: ::schedule::ResMut<crate::system::assembly::Jobs>,
    mut inbox: ::schedule::ResMut<Inbox>,
    mut control: ::schedule::ResMut<Control>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(crate::system::app::Fault::Room)?;
        let Incoming { wire, from, reply: reply_tx } = incoming;
        let (name, directive) = match wire {
            ccall::frame::Wire::Mint(name) => (
                name,
                Directive {
                    action: Action::Mint,
                    origin: Origin::External,
                },
            ),
            ccall::frame::Wire::Recover(name) => (
                name,
                Directive {
                    action: Action::Recover,
                    origin: Origin::Recovery { caller: from },
                },
            ),
            ccall::frame::Wire::Embark(name) => (
                name,
                Directive {
                    action: Action::Embark { parent: Some(from) },
                    origin: Origin::External,
                },
            ),
            ccall::frame::Wire::Debark(name) => (
                name,
                Directive {
                    action: Action::Debark,
                    origin: Origin::External,
                },
            ),
            ccall::frame::Wire::Ruin(name) => (
                name,
                Directive {
                    action: Action::Ruin,
                    origin: Origin::External,
                },
            ),
            ccall::frame::Wire::State(_) | ccall::frame::Wire::Task(_) => {
                return Err(crate::system::app::Fault::Room);
            }
            _ => {
                inbox.0.push_back(Incoming {
                    wire,
                    from,
                    reply: reply_tx,
                });
                continue;
            }
        };
        let is_ruin = directive.action == Action::Ruin;
        let reused = match operations.push_lifecycle(Request {
            name: name.clone(),
            directive,
            back: Some(reply_tx),
        }) {
            Ok((_, reused)) => reused,
            Err((fail, request)) => {
                if let Some(reply_tx) = request.back {
                    reply(reply_tx, status(fail));
                }
                continue;
            }
        };
        if reused && is_ruin {
            crate::debug::put(&alloc::format!(
                "system: lifecycle received target={} sender={} action=ruin",
                name,
                from.get(),
            ));
        }
        crate::system::control::lifecycle::accept_intent(
            &mut control,
            &name,
            if reused { Some(directive) } else { None },
        );
    }
    Ok(::schedule::Progress::Done)
}
fn status(fail: crate::system::control::unit::verdict::Fail) -> ccall::frame::Said {
    ccall::frame::said_status(ccall::frame::fail_to_code(Some(wire_fail(fail))))
}
pub(crate) fn reply(back: ReplySender<ccall::frame::Said>, said: ccall::frame::Said) {
    let _ = back.send(said);
}
fn wire_fail(fail: crate::system::control::unit::verdict::Fail) -> ccall::Fail {
    use crate::system::control::unit::verdict::Fail as Model;
    match fail {
        Model::Unknown => ccall::Fail::Unknown,
        Model::BadImage => ccall::Fail::BadImage,
        Model::Full => ccall::Fail::Full,
        Model::NotReady => ccall::Fail::NotReady,
        Model::Denied => ccall::Fail::Denied,
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
