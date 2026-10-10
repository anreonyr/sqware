use super::{Fail, answer::Output};
use crate::system::operator::session::{Desk, Gone, Guest};
use ::resource::pile::Pile;
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{MailCondition, PieToken, Wait};
use ipc::hand::{Receiver, RecvFail, Sender, SourceFail};
use system_api::operator as ocall;

pub(super) struct Outbox {
    pub reply: PieToken,
    pub send: Sender<ocall::Union>,
}
pub(super) struct Outboxes(pub Vec<Outbox>);
pub(super) struct Incoming {
    pub guest: Guest,
    pub ask: Option<ocall::Wire>,
}
pub(super) struct Request(pub Option<Incoming>);
pub(super) struct Buffer(pub Vec<u8>);
pub(super) struct Hit(pub Option<PieToken>);
pub(super) struct Selected(pub Option<Guest>);

pub(super) fn wait(pile: Res<Pile>, mut hit: ResMut<Hit>) -> Result<Progress, Fail> {
    hit.0 = pile
        .await_(Wait::AtMost(100))
        .map_err(|_| Fail::Dead)?
        .mail().map(|(token, _)| token);
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
        let decoded = match Receiver::<ocall::Req>::from_raw(guest.ask()).recv_from(
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
        });
    }
    Ok(Progress::Done)
}

pub(super) fn reply(
    mut request: ResMut<Request>,
    mut out: ResMut<Output<ocall::Union>>,
    mut outs: ResMut<Outboxes>,
) -> Result<Progress, Fail> {
    if let (Some(incoming), Some(reply)) = (request.0.take(), out.reply.take()) {
        let reply_token = incoming.guest.reply();
        let at = if let Some(at) = outs.0.iter().position(|out| out.reply == reply_token) {
            at
        } else {
            outs.0.try_reserve(1)?;
            outs.0.push(Outbox {
                reply: reply_token,
                send: Sender::from_raw(reply_token),
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
    let _ = desk.sweep_each(|gone: Gone| {
        let _ = pile.detach(env::Source::Mail { pie: gone.ask, condition: MailCondition::Pull });
        if let Some(at) = outs.0.iter().position(|out| out.reply == gone.reply) {
            outs.0.swap_remove(at);
        }
    });
    Ok(Progress::Done)
}
