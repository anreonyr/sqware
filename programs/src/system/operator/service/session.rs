use super::{Fail, answer::Output};
use crate::support::face::desk::{Desk, Guest};
use crate::system::operator::service::claim::{ask_of, mark_of};
use ::resource::pile::Pile;
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{HoleDir, Mark, PieToken, TaskId, Wait};
use ipc::hand::{Receiver, RecvFail, Sender, SourceFail};
use system_api::operator as ocall;
const SETTLE_MS: usize = 1;
const MARKS: [Mark; ocall::Grant::COUNT + 1] = {
    let mut marks = [ocall::ASK_MARK; ocall::Grant::COUNT + 1];
    let mut i = 0;
    while i < ocall::Grant::COUNT {
        marks[i + 1] = ocall::Grant::MARKS[i];
        i += 1;
    }
    marks
};
pub(super) struct Outbox {
    pub who: TaskId,
    pub send: Sender<ocall::Union>,
}
pub(super) struct Outboxes(pub Vec<Outbox>);
pub(super) struct Incoming {
    pub guest: Guest,
    pub ask: Option<ocall::Wire>,
    pub grant: Option<ocall::Grant>,
}
pub(super) struct Request(pub Option<Incoming>);
pub(super) struct Buffer(pub Vec<u8>);
pub(super) struct Hit(pub Option<PieToken>);
pub(super) struct Selected(pub Option<Guest>);
pub(super) struct Settling(pub bool);
pub(super) fn arm(
    mut desk: ResMut<Desk>,
    pile: Res<Pile>,
    mut settling: ResMut<Settling>,
) -> Result<Progress, Fail> {
    let mut rejected = Vec::new();
    let mut full = false;
    settling.0 = desk.arm_pending(
        |who| {
            for mark in MARKS {
                match ask_of(who, mark) {
                    Ok(token) => return Some(token),
                    Err(ipc::session::establish::DiscoveryFail::Missing) => {}
                    Err(ipc::session::establish::DiscoveryFail::Ambiguous) => {
                        if rejected.try_reserve(1).is_err() {
                            full = true;
                        } else {
                            rejected.push(who);
                        }
                        return None;
                    }
                }
            }
            None
        },
        |ask| pile.attach(ask, HoleDir::Pull).is_ok(),
    );
    if full {
        return Err(Fail::Room);
    }
    for who in rejected {
        desk.evict(who);
        programs::debug::put("operator: ambiguous guest request");
    }
    Ok(Progress::Done)
}
pub(super) fn wait(
    pile: Res<Pile>,
    settling: Res<Settling>,
    mut hit: ResMut<Hit>,
) -> Result<Progress, Fail> {
    hit.0 = pile
        .await_(Wait::AtMost(if settling.0 { SETTLE_MS } else { 100 }))
        .map_err(|_| Fail::Dead)?
        .map(|(token, _)| token);
    Ok(Progress::Done)
}
pub(super) fn select_guest(
    hit: Res<Hit>,
    desk: Res<Desk>,
    mut selected: ResMut<Selected>,
) -> Result<Progress, Fail> {
    selected.0 = hit.0.and_then(|token| desk.guest(token).copied());
    Ok(Progress::Done)
}
pub(super) fn receive(
    selected: Res<Selected>,
    mut buffer: ResMut<Buffer>,
    mut request: ResMut<Request>,
) -> Result<Progress, Fail> {
    request.0 = None;
    if let Some(guest) = selected.0 {
        if let Some(ask) = guest.ask() {
            let decoded = match Receiver::<ocall::Req>::from_raw(ask).recv_from(
                guest.who(),
                &mut buffer.0,
                Wait::POLL,
            ) {
                Ok(wire) => Some(wire),
                Err(SourceFail::Receive(RecvFail::Unread(_))) => None,
                Err(_) => return Ok(Progress::Done),
            };
            request.0 = Some(Incoming {
                guest,
                ask: decoded,
                grant: ocall::grant::grant_of(mark_of(ask)),
            });
        }
    }
    Ok(Progress::Done)
}
pub(super) fn reply(
    mut request: ResMut<Request>,
    mut out: ResMut<Output<ocall::Union>>,
    mut outs: ResMut<Outboxes>,
) -> Result<Progress, Fail> {
    if let (Some(incoming), Some(reply)) = (request.0.take(), out.reply.take()) {
        let guest = incoming.guest;
        let at = if let Some(at) = outs.0.iter().position(|out| out.who == guest.who()) {
            at
        } else {
            outs.0.try_reserve(1).map_err(|_| Fail::Room)?;
            outs.0.push(Outbox {
                who: guest.who(),
                send: Sender::from_raw(guest.reply()),
            });
            outs.0.len() - 1
        };
        if outs.0[at].send.settle() {
            let _ = outs.0[at].send.send(reply);
        }
    }
    Ok(Progress::Done)
}
pub(super) fn sweep(
    mut desk: ResMut<Desk>,
    pile: Res<Pile>,
    mut outs: ResMut<Outboxes>,
) -> Result<Progress, Fail> {
    let _ = desk.sweep_each(|gone| {
        if let Some(ask) = gone.ask {
            let _ = pile.detach(ask, HoleDir::Pull);
        }
        if let Some(at) = outs.0.iter().position(|out| out.who == gone.who) {
            if outs.0[at].send.settle() {
                outs.0.swap_remove(at);
            }
        }
    });
    Ok(Progress::Done)
}
