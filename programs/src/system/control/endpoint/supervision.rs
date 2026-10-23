use super::request::Buffer;
use crate::system::control::lifecycle::authority;
use crate::system::control::unit::Control;
use env::Wait;
use ipc::rpc::{self, request::Receiver as RequestReceiver};
use system_api::control::observe::{Call, Reply, Request};
use system_api::control::{self as ccall, Grant};

pub fn receive(
    endpoints: ::schedule::Res<crate::system::control::namespace::Endpoints>,
    mut buffer: ::schedule::ResMut<Buffer>,
    control: ::schedule::Res<Control>,
) -> Result<::schedule::Progress, crate::system::app::Fault> {
    let Some(face) = endpoints.face(Grant::Observe) else {
        return Ok(::schedule::Progress::Done);
    };
    let receiver = RequestReceiver::<Call>::from_raw(face, Call::BACK, Call::back);
    loop {
        let incoming = match receiver.receive(&mut buffer.0, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) => match rejected.fail {
                rpc::Fail::Receive(_) => break,
                _ => continue,
            },
        };
        let Request { index, .. } = incoming.request;
        let reply = match authority::targets(&control, incoming.from) {
            Ok(target) => view(&control, target.members, index),
            Err(_) => Reply::denied(index),
        };
        let _ = incoming.reply.send(reply);
    }
    Ok(::schedule::Progress::Done)
}

fn view(control: &Control, members: &'static [&'static str], index: u32) -> Reply {
    let Some(name) = members.get(index as usize).copied() else {
        return Reply::end(index);
    };
    let row = control.table.find(name);
    Reply {
        status: ccall::frame::OK,
        index,
        present: row.is_some(),
        end: false,
        run_requested: control.run_requested(name).unwrap_or(false),
        state: row.map_or(0, |row| super::request::wire_state(row.state).code()),
        name: name.into(),
    }
}
