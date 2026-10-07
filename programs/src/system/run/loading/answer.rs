use ::schedule::{Progress, ResMut};
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait, pie};
use protocol::{
    system::loader::frame::{self, Ask, Said, Wire},
};
use ::resource::raw::{Hole, inspect, reserve};
pub struct Incoming {
    pub ask: Ask,
    pub from: TaskId,
}
pub struct Inbox {
    pub entry: Option<PieToken>,
    pub requests: Vec<Incoming>,
    pub buffer: [u8; Ask::LEN],
}
impl Inbox {
    pub fn new() -> Self {
        Self {
            entry: None,
            requests: Vec::new(),
            buffer: [0; Ask::LEN],
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
        let Ok((n, from)) = Hole::from_raw(entry).pull(&mut inbox.buffer, Wait::POLL) else {
            break;
        };
        let ask = match Wire::take(&inbox.buffer[..n]) {
            Some(Wire::Build(ask)) => ask,
            Some(Wire::Claim(claim)) => {
                if !matches!(reserve(claim.back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == frame::BACK)
                {
                    continue;
                }
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
                if !reply(claim.back, said) && accepted.is_some() {
                    control.stop_instance(claim.task);
                }
                continue;
            }
            None => continue,
        };
        if !matches!(reserve(ask.back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == frame::BACK)
        {
            release_image(&ask, from);
            continue;
        }
        if inbox.requests.len() >= 16 || inbox.requests.try_reserve(1).is_err() {
            release_image(&ask, from);
            reply(
                ask.back,
                Said {
                    status: protocol::system::control::frame::FULL,
                    team: 0,
                    task: TaskId::new(0),
                },
            );
        } else {
            inbox.requests.push(Incoming { ask, from });
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
pub(super) fn reply(back: PieToken, said: Said) -> bool {
    let result = protocol::communication::hand::Sender::<Said>::from_raw(back)
        .send(said)
        .is_ok();
    let _ = pie::release(back);
    result
}

pub(super) fn reject(inbox: &mut Inbox) {
    for incoming in inbox.requests.drain(..) {
        release_image(&incoming.ask, incoming.from);
        reply(
            incoming.ask.back,
            Said {
                status: protocol::system::control::frame::NOTREADY,
                team: 0,
                task: TaskId::new(0),
            },
        );
    }
}
