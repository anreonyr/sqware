use runtime::schedule::{Progress, Res, ResMut};
use super::{Fail, answer::Output};
use crate::system::{
    common::face::desk::{Desk, Guest},
    operator::claim::{ask_of, mark_of, reply_of},
};
use alloc::vec::Vec;
use env::{HoleDir, Mark, PieToken, TaskId, Wait};
use protocol::{
    communication::hand::Sender,
    debug,
    system::operator as ocall,
};
use runtime::core::res::pie::HolePie;
use runtime::core::res::pile::Pile;
const SETTLE_MS: usize = 1;
const LATE_MS: usize = 1000;
const MARKS: [Mark; ocall::Grant::COUNT + 1] = {
    let mut marks = [ocall::ASK_MARK; ocall::Grant::COUNT + 1];
    let mut i = 0;
    while i < ocall::Grant::COUNT {
        marks[i + 1] = ocall::Grant::MARKS[i];
        i += 1;
    }
    marks
};
pub(super) struct Late {
    pub who: TaskId,
    pub since: u64,
}
pub(super) struct LateGuests(pub Vec<Late>);
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
pub(super) fn retry(
    mut desk: ResMut<Desk>,
    mut late: ResMut<LateGuests>,
    mut settling: ResMut<Settling>,
) -> Result<Progress, Fail> {
    let now = env::chrono::clock();
    let mut at = 0;
    while at < late.0.len() {
        let who = late.0[at].who;
        if let Some(reply) = reply_of(who) {
            let _ = desk.admit(who, reply);
            late.0.swap_remove(at);
        } else if now.saturating_sub(late.0[at].since) >= LATE_MS as u64 * 1_000_000 {
            debug!("operator: no reply who={} gave up", who.get());
            late.0.swap_remove(at);
        } else {
            at += 1;
        }
    }
    settling.0 = !late.0.is_empty();
    Ok(Progress::Done)
}
pub(super) fn arm(
    mut desk: ResMut<Desk>,
    pile: Res<Pile>,
    mut settling: ResMut<Settling>,
) -> Result<Progress, Fail> {
    settling.0 |= desk.arm_pending(
        |who| (&MARKS).iter().find_map(|mark| (ask_of)(who, *mark)),
        |ask| {
            pile.attach(ask, HoleDir::Pull)
                .is_ok()
        },
    );
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
            let decoded = match HolePie::from_token(ask).pull(&mut buffer.0, Wait::POLL) {
                Ok((n, from)) if from == guest.who() => {
                    <ocall::Req as protocol::wire::message::Message>::fetch(&buffer.0[..n])
                }
                Ok(_) => return Ok(Progress::Done),
                Err(_) => None,
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
                send: Sender::from_token(guest.reply()),
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
