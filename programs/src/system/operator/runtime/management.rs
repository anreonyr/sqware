use super::Fail;
use crate::system::app::life::Status;
use crate::system::operator::session::{Desk, DeskFail};
use crate::system::operator::{
    runtime::import::{valid_reply, valid_request},
    tree::Operator,
};
use ::resource::raw::{Hole, reserve};
use ::resource::{
    pile::Pile,
    port::{self, Access, Policy},
};
use ::schedule::{Dispatch, Invocation, Progress, Res, ResMut};
use alloc::{collections::VecDeque, sync::Arc};
use env::pie;
use env::{HoleCondition, PieToken, TaskId, ToleEventSource, Wait};
use programs::debug;
use system_api::operator as ocall;
use system_client::identity::TaskQuery;
pub(super) struct Tip(pub PieToken);
pub(super) struct Tips(pub VecDeque<(ocall::TipIn, TaskId)>);
pub(super) struct TipWork {
    current: Option<(ocall::TipIn, TaskId)>,
    reply: Option<Ack>,
}
impl TipWork {
    pub fn new() -> Self {
        Self {
            current: None,
            reply: None,
        }
    }
}
pub(super) struct Ack {
    pub back: PieToken,
    pub status: u8,
    pub id: u64,
}
pub(super) fn tip(
    status: Res<Arc<Status>>,
    mut tip: ResMut<Tip>,
    pile: Res<Pile>,
) -> Result<Progress, Fail> {
    tip.0 = env::hole::unseal(ocall::TIP_MARK, env::HoleLimits::default())
        .map_err(|_| Fail::Tree)?;
    let hole = Hole::from_raw(tip.0);
    port::ship(
        hole.token(),
        status.control,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    )
    .map_err(|_| Fail::Tree)?;
    pile.attach(ToleEventSource::Hole {
        token: hole.token(),
        condition: HoleCondition::Pull,
    })
    .map_err(|_| Fail::Desk)?;
    Ok(Progress::Done)
}
pub(super) fn receive_tips(
    status: Res<Arc<Status>>,
    tip: Res<Tip>,
    mut tips: ResMut<Tips>,
) -> Result<Progress, Fail> {
    let mut frame = [0; ocall::TIP_LEN];
    while let Ok((n, from)) = Hole::from_raw(tip.0).pull(&mut frame, Wait::POLL) {
        if from != status.control {
            debug::put("operator: foreign bootstrap tip");
            continue;
        }
        if let Some(tip) = ocall::TipIn::fetch(&frame[..n]) {
            tips.0.try_reserve(1)?;
            tips.0.push_back((tip, from));
        }
    }
    Ok(Progress::Done)
}
pub(super) fn budget(
    tips: Res<Tips>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    dispatch.begin(tips.0.len())?;
    Ok(Progress::Done)
}
pub(super) fn select(
    mut tips: ResMut<Tips>,
    mut work: ResMut<TipWork>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    work.current = tips.0.pop_front();
    if work.current.is_some() {
        dispatch.select(Invocation {
            key: (),
            cursor: Default::default(),
        })?;
    }
    Ok(Progress::Done)
}
pub(super) fn valid_tip_back(back: PieToken, from: TaskId) -> bool {
    matches!(reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == ocall::TIP_BACK)
}
pub(super) fn wired(
    mut tree: ResMut<Operator>,
    mut work: ResMut<TipWork>,
) -> Result<Progress, Fail> {
    work.reply = None;
    if let Some((
        ocall::TipIn::Wired {
            authority,
            resolve,
            matches,
            same,
            back,
        },
        from,
    )) = &work.current
    {
        if !valid_tip_back(*back, *from) {
            return Ok(Progress::Done);
        }
        tree.set_identity_query(TaskQuery::direct(*authority, *resolve, *matches, *same).ok());
        work.reply = Some(Ack {
            back: *back,
            status: if tree.identity_query().is_some() {
                ocall::OK
            } else {
                ocall::UNJUDGED
            },
            id: 0,
        });
    }
    Ok(Progress::Done)
}
pub(super) fn guest(
    work: Res<TipWork>,
    mut desk: ResMut<Desk>,
    pile: Res<Pile>,
) -> Result<Progress, Fail> {
    let Some((ocall::TipIn::Guest { who, reply, ask }, from)) = &work.current else {
        return Ok(Progress::Done);
    };
    if !valid_reply(*who, *reply, *from) {
        debug::put("operator: invalid explicit guest reply");
        return Ok(Progress::Done);
    }
    if !valid_request(*who, *ask, *who) {
        if !desk.contains_reply(*reply) {
            let _ = Hole::from_raw(*reply).push(&[ocall::DENIED], Wait::POLL);
        }
        return Ok(Progress::Done);
    }
    match desk.admit(*who, (*reply, *ask)) {
        Ok(_) => {}
        Err(DeskFail::Already) => return Ok(Progress::Done),
        Err(DeskFail::Conflict) => return Ok(Progress::Done),
        Err(DeskFail::Full) => {
            let _ = Hole::from_raw(*reply).push(&[ocall::DENIED], Wait::POLL);
            return Ok(Progress::Done);
        }
    }
    if pile
        .attach(ToleEventSource::Hole {
            token: *ask,
            condition: HoleCondition::Pull,
        })
        .is_err()
    {
        desk.evict(*who, (*reply, *ask));
        let _ = Hole::from_raw(*reply).push(&[ocall::DENIED], Wait::POLL);
        return Ok(Progress::Done);
    }
    if Hole::from_raw(*reply)
        .push(&[ocall::OK], Wait::POLL)
        .is_err()
    {
        debug::put("operator: guest acknowledgement failed");
        desk.evict(*who, (*reply, *ask));
        let _ = pile.detach(ToleEventSource::Hole {
            token: *ask,
            condition: HoleCondition::Pull,
        });
    }
    Ok(Progress::Done)
}
pub(super) fn root(
    mut work: ResMut<TipWork>,
    mut tree: ResMut<Operator>,
    pile: Res<Pile>,
) -> Result<Progress, Fail> {
    let Some((rec, from)) = work.current.take() else {
        return Ok(Progress::Done);
    };
    match rec {
        ocall::TipIn::Root { back } => {
            if from != tree.system_task() || !valid_tip_back(back, from) {
                return Ok(Progress::Done);
            }
            let existing = tree.root_entrance();
            let entrance = match existing {
                Some(token) => token,
                None => env::hole::unseal(ocall::entry::AUTH_ENTRY, env::HoleLimits::default())
                    .map_err(|_| Fail::Tree)?,
            };
            let hole = Hole::from_raw(entrance);
            let remote = match port::ship(hole.token(), from, Access::STORE, Policy::NONE) {
                Ok(remote) => remote.seed(),
                Err(_) => {
                    if existing.is_none() {
                        let _ = env::hole::seal(entrance);
                        let _ = pie::release(entrance, env::ReleaseMode::Revoke);
                    }
                    return Ok(Progress::Done);
                }
            };
            if existing.is_none()
                && (pile
                    .attach(ToleEventSource::Hole {
                        token: entrance,
                        condition: HoleCondition::Pull,
                    })
                    .is_err()
                    || tree.bootstrap_root(from, entrance).is_err())
            {
                let _ = pile.detach(ToleEventSource::Hole {
                    token: entrance,
                    condition: HoleCondition::Pull,
                });
                let _ = env::hole::seal(entrance);
                let _ = pie::release(entrance, env::ReleaseMode::Revoke);
                return Ok(Progress::Done);
            }
            work.reply = Some(Ack {
                back,
                status: ocall::OK,
                id: remote.get() as u64,
            });
        }
        _ => {}
    }
    Ok(Progress::Done)
}
pub(super) fn acknowledge(mut work: ResMut<TipWork>) -> Result<Progress, Fail> {
    if let Some(Ack { back, status, id }) = work.reply.take() {
        let mut bytes = [0; 9];
        bytes[0] = status;
        bytes[1..].copy_from_slice(&id.to_le_bytes());
        let _ = Hole::from_raw(back).push(&bytes, Wait::AtMost(1000));
        let _ = pie::release(back, env::ReleaseMode::Revoke);
    }
    Ok(Progress::Done)
}
pub(super) fn finish(mut dispatch: ResMut<Dispatch<(), Fail>>) -> Result<Progress, Fail> {
    if dispatch.take_result()?.result.is_err() {
        return Err(Fail::Tree);
    }
    Ok(Progress::Done)
}
pub(super) fn close(tip: Res<Tip>, pile: Res<Pile>) -> Result<Progress, Fail> {
    let _ = pile.detach(ToleEventSource::Hole {
        token: tip.0,
        condition: HoleCondition::Pull,
    });
    let _ = env::hole::seal(tip.0);
    let _ = pie::release(tip.0, env::ReleaseMode::Revoke);
    Ok(Progress::Done)
}
