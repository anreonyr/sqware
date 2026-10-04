//! 一份由装配者接入的 Resolve / Matches / Same 查询束；不按 mark 猜权威。

use env::{TaskId, Wait};
use protocol::system::identity::client::TaskQuery;
use protocol::system::identity::{Match, Selector};
use protocol::system::operator::{EntryId, Permit};

use crate::system::operator::core::Operator;
use crate::system::operator::core::gate::{Code, verdict};
use crate::system::operator::core::judge::Facts;

const MS: usize = 1000;

fn matched(answer: Match) -> bool {
    match answer {
        Match::Yes => true,
        Match::No | Match::Unbound => false,
    }
}

struct Court<'a> {
    query: Option<&'a TaskQuery>,
    tree: &'a Operator,
}

impl Facts for Court<'_> {
    fn bound(&self, task: TaskId) -> Result<bool, ()> {
        self.query
            .ok_or(())?
            .resolve(task, Wait::AtMost(MS))
            .map(|binding| binding.is_some())
            .map_err(|_| ())
    }

    fn matches(&self, task: TaskId, selector: Selector) -> Result<bool, ()> {
        self.query
            .ok_or(())?
            .matches(task, selector, Wait::AtMost(MS))
            .map(matched)
            .map_err(|_| ())
    }

    fn same(&self, a: TaskId, b: TaskId) -> Result<bool, ()> {
        self.query
            .ok_or(())?
            .same(a, b, Wait::AtMost(MS))
            .map(matched)
            .map_err(|_| ())
    }

    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()> {
        self.tree.opens(at).map(Some).map_err(|_| ())
    }
}

pub(super) fn authorize(
    tree: protocol::common::schedule::Res<Operator>,
    query: protocol::common::schedule::Res<Option<TaskQuery>>,
    mut judgment: protocol::common::schedule::ResMut<Judgment>,
) -> Result<protocol::common::schedule::Progress, super::Fail> {
    if let Some((who, permit, ruling)) = judgment.0 {
        if ruling.passed() {
            judgment.0 = Some((
                who,
                permit,
                verdict(
                    &Court {
                        tree: &tree,
                        query: query.as_ref(),
                    },
                    who,
                    permit,
                ),
            ));
        }
    }
    Ok(protocol::common::schedule::Progress::Done)
}

use super::{answer::Output, session::Request};
use crate::system::life::Status;
use alloc::sync::Arc;
use protocol::{
    common::schedule::{Progress, Res, ResMut},
    system::operator as ocall,
};
pub(super) struct Judgment(pub Option<(TaskId, Permit, Code)>);
pub(super) fn validate(
    status: Res<Arc<Status>>,
    mut request: ResMut<Request>,
    mut out: ResMut<Output<ocall::Union>>,
) -> Result<Progress, super::Fail> {
    out.reply = None;
    out.changes.clear();
    let Some(incoming) = &mut request.0 else {
        return Ok(Progress::Done);
    };
    let denied = incoming.ask.as_ref().is_some_and(|ask| {
        incoming
            .grant
            .is_some_and(|grant| grant.at() != ocall::Grant::of_wire(ask))
            || (matches!(
                ask,
                ocall::Wire::Part { .. } | ocall::Wire::Land { .. } | ocall::Wire::Trim(_)
            ) && incoming.guest.who() != status.control)
    });
    if denied {
        incoming.ask = None;
        out.reply = Some(ocall::Union::Status(ocall::DENIED));
    } else if incoming.ask.is_none() {
        out.reply = Some(ocall::Union::Status(ocall::BAD));
    }
    Ok(Progress::Done)
}
pub(super) fn permit(
    request: Res<Request>,
    tree: Res<Operator>,
    mut judgment: ResMut<Judgment>,
) -> Result<Progress, super::Fail> {
    judgment.0 = None;
    let Some(incoming) = &request.0 else {
        return Ok(Progress::Done);
    };
    let Some(ask) = &incoming.ask else {
        return Ok(Progress::Done);
    };
    let who = incoming.guest.who();
    let permit = match ask {
        ocall::Wire::Find(id) => tree.permit(*id),
        ocall::Wire::Trim(id) => {
            if !tree.claimable(crate::system::operator::core::Key::Id(*id), who) {
                judgment.0 = Some((who, ocall::Permit::Bound, Code::Denied));
                return Ok(Progress::Done);
            }
            ocall::Permit::Bound
        }
        ocall::Wire::Land { .. } | ocall::Wire::Part { .. } | ocall::Wire::Watch { .. } => {
            ocall::Permit::Bound
        }
        _ => return Ok(Progress::Done),
    };
    judgment.0 = Some((who, permit, Code::Ok));
    Ok(Progress::Done)
}
pub(super) fn admit(
    mut request: ResMut<Request>,
    judgment: Res<Judgment>,
    mut out: ResMut<Output<ocall::Union>>,
) -> Result<Progress, super::Fail> {
    if let Some((_, _, ruling)) = judgment.0 {
        if !ruling.passed() {
            if let Some(incoming) = &mut request.0 {
                incoming.ask = None;
            }
            out.reply = Some(ocall::Union::Status(ruling.wire()));
        }
    }
    Ok(Progress::Done)
}
