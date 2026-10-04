use crate::system::control::core::unit::State;
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait, pie};
use protocol::{
    common::schedule::{Progress, ResMut},
    system::loader::frame::{self, Ask, Said, Wire},
};
use runtime::core::res::pie::{HolePie, inspect, reserve};
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
        let Ok((n, from)) = HolePie::from_token(entry).pull(&mut inbox.buffer, Wait::POLL) else {
            break;
        };
        let ask = match Wire::take(&inbox.buffer[..n]) {
            Some(Wire::Build(ask)) => ask,
            Some(Wire::Claim(claim)) => {
                if !matches!(reserve(claim.back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == frame::BACK)
                {
                    continue;
                }
                let mut accepted = None;
                if let Some(item) = control
                    .instances
                    .iter_mut()
                    .find(|item| item.task == claim.task && item.owner == from)
                {
                    if env::chrono::clock() < item.claim_until && item.state == State::Debarked {
                        if let Some(team) = item.team {
                            item.claimed = true;
                            accepted = Some(team);
                        }
                    }
                }
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
                    if let Some(item) = control
                        .instances
                        .iter_mut()
                        .find(|item| item.task == claim.task)
                    {
                        item.state = State::Stopping;
                    }
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
    let result = protocol::communication::hand::Sender::<Said>::from_token(back)
        .send(said)
        .is_ok();
    let _ = pie::release(back);
    result
}
