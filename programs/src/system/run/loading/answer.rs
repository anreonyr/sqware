use ::schedule::{Progress, ResMut};
use alloc::vec::Vec;
use env::{MailFail, PieToken, TaskId, Wait, pie};
use protocol::system::loader::frame::{self, Ask, Said, Wire};
use protocol::wire::message::Message;
use ipc::rpc::{self, ReplyTo};
use ::resource::raw::inspect;

pub struct Incoming {
    pub ask: Ask,
    pub from: TaskId,
    pub back: ReplyTo,
}
pub struct Inbox {
    pub entry: Option<PieToken>,
    pub requests: Vec<Incoming>,
    pub buffer: <Wire as Message>::Buf,
}
impl Inbox {
    pub fn new() -> Self {
        Self {
            entry: None,
            requests: Vec::new(),
            buffer: Wire::EMPTY,
        }
    }
}
pub fn receive(
    mut inbox: ResMut<Inbox>,
    mut control: ResMut<crate::system::control::serve::unit::Control>,
) -> Result<Progress, crate::system::control::serve::Fail> {
    let Some(entry) = inbox.entry else {
        return Ok(Progress::Done);
    };
    for _ in 0..16 {
        let incoming = match rpc::receive::<Wire>(entry, &mut inbox.buffer, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rpc::ReceiveFail::Mail(MailFail::Busy)) => break,
            Err(rpc::ReceiveFail::Mail(_)) => break,
            Err(rpc::ReceiveFail::Malformed(_)) => continue,
        };
        let from = incoming.from;
        match incoming.request {
            Wire::Claim(claim) => {
                let Ok(back) = ReplyTo::from_raw(claim.back, from, frame::BACK) else {
                    continue;
                };
                let accepted = control.claim_instance(from, claim.task);
                let said = Said {
                    status: if accepted.is_some() {
                        protocol::wire::OK
                    } else {
                        protocol::system::control::frame::NOTREADY
                    },
                    team: accepted.map_or(0, |team| team.get() as u64),
                    task: claim.task,
                };
                if back.send(said).is_err() && accepted.is_some() {
                    control.stop_instance(claim.task);
                }
            }
            Wire::Build(ask) => {
                let Ok(back) = ReplyTo::from_raw(ask.back, from, frame::BACK) else {
                    release_image(&ask, from);
                    continue;
                };
                if inbox.requests.len() >= 16 || inbox.requests.try_reserve(1).is_err() {
                    release_image(&ask, from);
                    reply(back, Said {
                        status: protocol::system::control::frame::FULL,
                        team: 0,
                        task: TaskId::new(0),
                    });
                } else {
                    inbox.requests.push(Incoming { ask, from, back });
                }
            }
        }
    }
    Ok(Progress::Done)
}
pub(super) fn release_image(ask: &Ask, from: TaskId) {
    if matches!(inspect(ask.image), Ok((vestor, _, mark)) if vestor == from && mark == frame::IMAGE)
    {
        let _ = pie::release(ask.image);
    }
}
pub(super) fn reply(back: ReplyTo, said: Said) -> bool {
    back.send(said).is_ok()
}

pub(super) fn reject(inbox: &mut Inbox) {
    for incoming in inbox.requests.drain(..) {
        release_image(&incoming.ask, incoming.from);
        reply(
            incoming.back,
            Said {
                status: protocol::system::control::frame::NOTREADY,
                team: 0,
                task: TaskId::new(0),
            },
        );
    }
}
