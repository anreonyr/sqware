use super::answer::{self, Inbox};
use crate::system::app::Fault as Fail;
use crate::system::control::instance::Command;
use crate::system::control::unit::Control;
use ::schedule::{Progress, Res, ResMut};
use env::Wait;
use ipc::rpc::{self, request::Receiver as RequestReceiver};
use system_api::control as call;
use system_api::control::Call as ControlContract;

pub fn answer(mut control: ResMut<Control>, mut inbox: ResMut<Inbox>) -> Result<Progress, Fail> {
    let count = inbox.0.len();
    for _ in 0..count {
        let incoming = inbox.0.pop_front().ok_or(Fail::Room)?;
        let command = match &incoming.wire {
            call::frame::Wire::EmbarkInstance(task) => Command::Embark(*task),
            call::frame::Wire::DebarkInstance(task) => Command::Debark(*task),
            call::frame::Wire::RuinInstance(task) => Command::Ruin(*task),
            call::frame::Wire::StateInstance(task) => Command::State(*task),
            _ => {
                inbox.0.push_back(incoming);
                continue;
            }
        };
        if !incoming.reply.is_alive() {
            drop(incoming.reply);
            continue;
        }
        let result = control.command_instance(incoming.from, command);
        match result {
            Ok(Some(state)) => {
                let said = if matches!(command, Command::State(_)) {
                    call::frame::said_state(answer::wire_state(state))
                } else {
                    call::frame::said_status(call::frame::OK)
                };
                answer::reply(incoming.reply, said);
            }
            Ok(None) => inbox.0.push_back(incoming),
            Err(fail) => answer::reply(
                incoming.reply,
                call::frame::said_status(call::frame::fail_to_code(Some(fail))),
            ),
        }
    }
    Ok(Progress::Done)
}

pub fn receive(
    watch: Res<super::watch::Watch>,
    mut buffer: ResMut<answer::Buffer>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    let Some(entry) = watch.instance else {
        return Ok(Progress::Done);
    };
    let receiver = RequestReceiver::<ControlContract>::from_raw(
        entry,
        ControlContract::BACK,
        ControlContract::back,
    );
    for _ in 0..16 {
        let request = match receiver.receive(&mut buffer.0, Wait::POLL) {
            Ok(request) => request,
            Err(rejected) => match rejected.fail {
                rpc::Fail::Receive(_) => break,
                _ => continue,
            },
        };
        let from = request.from;
        let (wire, _) = request.request;
        let reply = request.reply;
        match wire {
            Some(
                wire @ (call::frame::Wire::EmbarkInstance(_)
                | call::frame::Wire::DebarkInstance(_)
                | call::frame::Wire::RuinInstance(_)
                | call::frame::Wire::StateInstance(_)),
            ) => {
                if inbox.0.try_reserve(1).is_err() {
                    answer::reply(reply, call::frame::said_status(call::frame::FULL));
                } else {
                    inbox.0.push_back(answer::Incoming { wire, from, reply });
                }
            }
            _ => answer::reply(reply, call::frame::said_status(call::frame::DENIED)),
        }
    }
    Ok(Progress::Done)
}
