use ::resource::raw::inspect;
use ::schedule::{Progress, ResMut};
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait, pie};
use ipc::rpc::{self, reply::Sender};
use system_api::loader as frame;
use system_api::loader::Ask;
use system_api::loader::Call as Contract;
use system_api::loader::Said;
use system_api::loader::Wire;
use wire::Message;

pub(crate) struct Incoming {
    pub(crate) ask: Ask,
    pub(crate) from: TaskId,
    pub(crate) back: Sender<Said>,
}
pub(crate) struct Inbox {
    pub(crate) entry: Option<PieToken>,
    pub(crate) requests: Vec<Incoming>,
    pub(crate) buffer: <Wire as Message>::Buf,
}
impl Inbox {
    pub(crate) fn new() -> Self {
        Self {
            entry: None,
            requests: Vec::new(),
            buffer: Wire::EMPTY,
        }
    }
}
pub(super) fn receive(
    mut inbox: ResMut<Inbox>,
    mut control: ResMut<crate::system::control::unit::Control>,
) -> Result<Progress, crate::system::app::Fault> {
    let Some(entry) = inbox.entry else {
        return Ok(Progress::Done);
    };
    let receiver =
        rpc::request::Receiver::<Contract>::from_raw(entry, Contract::BACK, Contract::back);
    for _ in 0..16 {
        let incoming = match receiver.receive(&mut inbox.buffer, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) => {
                if let Some((from, Wire::Build(ask))) = rejected.incoming {
                    release_image(&ask, from);
                }
                if matches!(rejected.fail, rpc::Fail::Receive(_)) {
                    break;
                }
                continue;
            }
        };
        let from = incoming.from;
        let back = incoming.reply;
        match incoming.request {
            Wire::Claim(claim) => {
                let accepted = control.claim_instance(from, claim.task);
                let said = Said {
                    status: if accepted.is_some() {
                        wire::OK
                    } else {
                        system_api::control::frame::NOTREADY
                    },
                    team: accepted.map_or(0, |team| team.get() as u64),
                    task: claim.task,
                };
                if back.send(said).is_err() && accepted.is_some() {
                    control.stop_instance(claim.task);
                }
            }
            Wire::Build(ask) => {
                if inbox.requests.len() >= 16 || inbox.requests.try_reserve(1).is_err() {
                    release_image(&ask, from);
                    reply(
                        back,
                        Said {
                            status: system_api::control::frame::FULL,
                            team: 0,
                            task: TaskId::new(0),
                        },
                    );
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
pub(super) fn reply(back: Sender<Said>, said: Said) -> bool {
    back.send(said).is_ok()
}

pub(super) fn reject(inbox: &mut Inbox) {
    for incoming in inbox.requests.drain(..) {
        release_image(&incoming.ask, incoming.from);
        reply(
            incoming.back,
            Said {
                status: system_api::control::frame::NOTREADY,
                team: 0,
                task: TaskId::new(0),
            },
        );
    }
}
