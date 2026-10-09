use ::resource::raw::inspect;
use ::schedule::{Progress, ResMut};
use env::{PieToken, TaskId, Wait, pie};
use ipc::rpc::{self, reply::Sender};
use system_api::loader as frame;
use system_api::loader::Ask;
use system_api::loader::Call as Contract;
use system_api::loader::Said;
use system_api::loader::Wire;
use wire::Message;

pub(crate) struct Inbox {
    pub(crate) entry: Option<PieToken>,
    pub(crate) buffer: <Wire as Message>::Buf,
}
impl Inbox {
    pub(crate) fn new() -> Self {
        Self {
            entry: None,
            buffer: Wire::EMPTY,
        }
    }
}
pub(super) fn receive(
    mut inbox: ResMut<Inbox>,
    mut control: ResMut<crate::system::control::unit::Control>,
    mut requests: ResMut<crate::system::launch::Requests>,
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
                if requests.0.len() >= 16 || requests.0.try_reserve(1).is_err() {
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
                    requests.0.push(crate::system::launch::Request { ask, from, delivery: crate::system::launch::Delivery { owner: from, identity: system_api::identity::Install::Inherit { parent: from }, constructor: false, back } });
                }
            }
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn release_image(ask: &Ask, from: TaskId) {
    if matches!(inspect(ask.image), Ok(info) if info.alive && info.vestor == from && info.mark == frame::IMAGE)
    {
        let _ = pie::release(ask.image, env::ReleaseMode::Revoke);
    }
}
pub(super) fn reply(back: Sender<Said>, said: Said) -> bool {
    back.send(said).is_ok()
}
