use super::{
    Fail,
    answer::Output,
    plate,
    session::{Late, LateGuests},
};
use crate::support::face::desk::{Desk, DeskFail};
use crate::system::app::life::Status;
use crate::system::operator::tree::Tile;
use crate::system::operator::{service::claim::reply_of, tree::Operator};
use ::resource::raw::{Hole, reserve};
use ::resource::{
    pile::Pile,
    port::{self, Access, Policy},
};
use ::schedule::{Dispatch, Invocation, Progress, Res, ResMut};
use alloc::{collections::VecDeque, sync::Arc};
use env::pie;
use env::{HoleDir, PieToken, TaskId, Wait};
use programs::debug;
use system_api::operator as ocall;
use system_client::identity::TaskQuery;
pub(super) struct Tip(pub PieToken);
pub(super) struct Tips(pub VecDeque<(ocall::TipIn, TaskId)>);
pub(super) struct CurrentTip(pub Option<(ocall::TipIn, TaskId)>);
pub(super) struct Ack {
    pub back: PieToken,
    pub status: u8,
    pub id: ocall::EntryId,
}
pub(super) fn tip(
    status: Res<Arc<Status>>,
    mut tip: ResMut<Tip>,
    pile: Res<Pile>,
) -> Result<Progress, Fail> {
    tip.0 = pie::unseal_hole(ocall::TIP_MARK).map_err(|_| Fail::Tree)?;
    let hole = Hole::from_raw(tip.0);
    port::ship(
        hole.token(),
        status.control,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    )
    .map_err(|_| Fail::Tree)?;
    pile.attach(hole.token(), HoleDir::Pull)
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
            tips.0.try_reserve(1).map_err(|_| Fail::Room)?;
            tips.0.push_back((tip, from));
        }
    }
    Ok(Progress::Done)
}
pub(super) fn budget(
    tips: Res<Tips>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    dispatch.begin(tips.0.len()).map_err(|_| Fail::Room)?;
    Ok(Progress::Done)
}
pub(super) fn select(
    mut tips: ResMut<Tips>,
    mut current: ResMut<CurrentTip>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    current.0 = tips.0.pop_front();
    if current.0.is_some() {
        dispatch
            .select(Invocation {
                key: (),
                cursor: Default::default(),
            })
            .map_err(|_| Fail::Room)?;
    }
    Ok(Progress::Done)
}
pub(super) fn valid_tip_back(back: PieToken, from: TaskId) -> bool {
    matches!(reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == ocall::TIP_BACK)
}
pub(super) fn wired(
    current: Res<CurrentTip>,
    mut query: ResMut<Option<TaskQuery>>,
    mut out: ResMut<Output<Ack>>,
) -> Result<Progress, Fail> {
    out.reply = None;
    out.changes.clear();
    if let Some((
        ocall::TipIn::Wired {
            authority,
            resolve,
            matches,
            same,
            back,
        },
        from,
    )) = &current.0
    {
        if !valid_tip_back(*back, *from) {
            return Ok(Progress::Done);
        }
        *query = TaskQuery::direct(*authority, *resolve, *matches, *same).ok();
        out.reply = Some(Ack {
            back: *back,
            status: if query.is_some() {
                ocall::OK
            } else {
                ocall::UNJUDGED
            },
            id: ocall::EntryId::new(0),
        });
    }
    Ok(Progress::Done)
}
pub(super) fn guest(
    current: Res<CurrentTip>,
    mut desk: ResMut<Desk>,
    mut late: ResMut<LateGuests>,
) -> Result<Progress, Fail> {
    if let Some((ocall::TipIn::Guest(client), _)) = &current.0 {
        if let Some(reply) = reply_of(*client) {
            match desk.admit(*client, reply) {
                Ok(_) | Err(DeskFail::Already) => {}
                Err(DeskFail::Full) => debug::put("operator: desk full"),
            }
        } else if !late.0.iter().any(|one| one.who == *client) {
            late.0.try_reserve(1).map_err(|_| Fail::Room)?;
            late.0.push(Late {
                who: *client,
                since: env::chrono::clock(),
            });
        }
    }
    Ok(Progress::Done)
}
pub(super) fn mutate(
    mut current: ResMut<CurrentTip>,
    mut tree: ResMut<Operator>,
    mut out: ResMut<Output<Ack>>,
) -> Result<Progress, Fail> {
    let Some((rec, from)) = current.0.take() else {
        return Ok(Progress::Done);
    };
    match rec {
        ocall::TipIn::Plate {
            road,
            leaf,
            permit,
            owner,
            replace,
            back,
        } => {
            if !valid_tip_back(back, from) {
                return Ok(Progress::Done);
            }
            match plate::plate(
                &mut tree,
                &crate::system::operator::Placement {
                    road,
                    tile: Tile {
                        pie: leaf,
                        permit,
                        owner: (owner.get() != 0).then_some(owner),
                    },
                    replace,
                },
            ) {
                Ok((id, changes)) => {
                    out.changes.extend(changes);
                    out.reply = Some(Ack {
                        back,
                        status: ocall::OK,
                        id,
                    });
                }
                Err(fail) => {
                    out.reply = Some(Ack {
                        back,
                        status: ocall::fail_to_code(Some(fail)),
                        id: ocall::EntryId::new(0),
                    })
                }
            }
        }
        ocall::TipIn::Abort { road, leaf, back } => {
            if !valid_tip_back(back, from) {
                return Ok(Progress::Done);
            }
            let result = match tree.seek(&road) {
                Ok(id) if tree.reference(id) == Some(leaf) => tree.trim(id),
                _ => Ok(None),
            };
            let fail = match result {
                Ok(change) => {
                    out.changes.extend(change);
                    None
                }
                Err(fail) => Some(fail),
            };
            out.reply = Some(Ack {
                back,
                status: ocall::fail_to_code(fail),
                id: ocall::EntryId::new(0),
            });
        }
        ocall::TipIn::Empty { road, back } => {
            if !valid_tip_back(back, from) {
                return Ok(Progress::Done);
            }
            let result = match tree.seek(&road) {
                Ok(id) => tree
                    .list(ocall::Where::At(id))
                    .and_then(|children| {
                        if children.count() == 0 {
                            Ok(id)
                        } else {
                            Err(ocall::Fail::NonEmpty)
                        }
                    })
                    .and_then(|id| tree.trim(id).map(|_| id)),
                Err(ocall::Fail::Unknown) => Ok(ocall::EntryId::new(0)),
                Err(fail) => Err(fail),
            };
            out.reply = Some(Ack {
                back,
                status: ocall::fail_to_code(result.err()),
                id: ocall::EntryId::new(0),
            });
        }
        ocall::TipIn::Unplate { id, back } => {
            if !valid_tip_back(back, from) {
                return Ok(Progress::Done);
            }
            let result = match tree.trim(id) {
                Err(ocall::Fail::Unknown) => Ok(None),
                other => other,
            };
            let fail = match result {
                Ok(change) => {
                    out.changes.extend(change);
                    None
                }
                Err(fail) => Some(fail),
            };
            out.reply = Some(Ack {
                back,
                status: ocall::fail_to_code(fail),
                id,
            });
        }
        _ => {}
    }
    Ok(Progress::Done)
}
pub(super) fn acknowledge(mut out: ResMut<Output<Ack>>) -> Result<Progress, Fail> {
    if let Some(Ack { back, status, id }) = out.reply.take() {
        let mut bytes = [0; 9];
        bytes[0] = status;
        bytes[1..].copy_from_slice(&(id.get() as u64).to_le_bytes());
        let _ = Hole::from_raw(back).push(&bytes, Wait::AtMost(1000));
        let _ = pie::release(back);
    }
    Ok(Progress::Done)
}
pub(super) fn finish(mut dispatch: ResMut<Dispatch<(), Fail>>) -> Result<Progress, Fail> {
    if dispatch
        .take_result()
        .map_err(|_| Fail::Room)?
        .result
        .is_err()
    {
        return Err(Fail::Tree);
    }
    Ok(Progress::Done)
}
pub(super) fn close(tip: Res<Tip>, pile: Res<Pile>) -> Result<Progress, Fail> {
    let _ = pile.detach(tip.0, HoleDir::Pull);
    let _ = pie::seal(tip.0);
    let _ = pie::release(tip.0);
    Ok(Progress::Done)
}
