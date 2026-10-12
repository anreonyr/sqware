use ::schedule::{Progress, Res, ResMut};
use env::{PieToken, Wait};
use ipc::rpc;
use resource::raw;
use system_api::loader::Call as Contract;
use system_api::loader::Said;
use system_api::loader::Wire;
use wire::Message;

pub(crate) struct Inbox {
    pub(crate) entry: Option<PieToken>,
    pub(crate) buffer: <Wire as Message>::Buf,
}

pub(crate) fn release_image(image: &system_api::control::construction::Image, from: env::TaskId) {
    if matches!(raw::inspect(image.image), Ok(info)
        if info.alive && info.vestor == from && info.mark == system_api::loader::IMAGE)
    {
        let _ = env::pie::release(image.image, env::ReleaseMode::Revoke);
    }
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
    mut endpoints: ResMut<super::namespace::EndpointState>,
    mut control: ResMut<crate::system::control::unit::Control>,
    jobs: Res<crate::system::assembly::Jobs>,
) -> Result<Progress, crate::system::app::Fault> {
    let inbox = &mut endpoints.inbox;
    let Some(entry) = inbox.entry else {
        return Ok(Progress::Done);
    };
    let receiver =
        rpc::request::Receiver::<Contract>::from_raw(entry, Contract::BACK, Contract::back);
    for _ in 0..16 {
        let incoming = match receiver.receive(&mut inbox.buffer, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) => {
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
                let startup = if accepted.is_some() {
                    jobs.startup(claim.task)
                        .unwrap_or(system_api::loader::StartupMetadata {
                            resolve: env::PieToken::NONE,
                        })
                } else {
                    system_api::loader::StartupMetadata {
                        resolve: env::PieToken::NONE,
                    }
                };
                let said = Said {
                    status: if accepted.is_some() {
                        wire::OK
                    } else {
                        system_api::control::frame::NOTREADY
                    },
                    state: if accepted.is_some() {
                        system_api::control::construction::COMPLETED
                    } else {
                        system_api::control::construction::FAILED
                    },
                    team: accepted.map_or(0, |team| team.get() as u64),
                    task: claim.task,
                    startup,
                };
                let _ = back.send(said);
            }
        }
    }
    Ok(Progress::Done)
}
