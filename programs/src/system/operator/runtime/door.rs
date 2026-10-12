//! 一份由装配者接入的 Resolve / Matches / Same 查询束；不按 mark 猜权威。

use ::schedule::{Progress, Res, ResMut};

use env::{TaskId, Wait};
use system_api::identity::Match;
use system_api::identity::Selector;
use system_api::operator::EntryId;
use system_api::operator::Permit;
use system_client::identity::TaskQuery;

use crate::system::operator::tree::Facts;
use crate::system::operator::tree::Operator;
use crate::system::operator::tree::{Code, verdict};

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
    tree: ::schedule::Res<Operator>,
    mut judgment: ::schedule::ResMut<Judgment>,
) -> Result<::schedule::Progress, super::Fail> {
    if let Some((who, permit, ruling)) = judgment.0 {
        if ruling.passed() {
            judgment.0 = Some((
                who,
                permit,
                verdict(
                    &Court {
                        tree: &tree,
                        query: tree.identity_query(),
                    },
                    who,
                    permit,
                ),
            ));
        }
    }
    Ok(::schedule::Progress::Done)
}

use super::{answer::Output, exchange::Request};
use system_api::operator as ocall;
pub(super) struct Judgment(pub Option<(TaskId, Permit, Code)>);
pub(super) fn validate(
    mut request: ResMut<Request>,
    mut out: ResMut<Output<ocall::Union>>,
) -> Result<Progress, super::Fail> {
    out.reply = None;
    out.changes.clear();
    let Some(incoming) = &mut request.0 else {
        return Ok(Progress::Done);
    };
    if incoming.ask.is_none() {
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
        ocall::Wire::Watch { .. } => ocall::Permit::Bound,
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
