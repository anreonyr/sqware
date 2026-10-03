//! 一份由装配者接入的 Resolve / Matches / Same 查询束；不按 mark 猜权威。

use env::{TaskId, Wait};
use protocol::service::identity::client::TaskQuery;
use protocol::service::identity::{Match, Selector};
use protocol::service::operator::{EntryId, Permit};

use crate::service::operator::core::Operator;
use crate::service::operator::core::gate::{Code, verdict};
use crate::service::operator::core::judge::Facts;

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

pub(super) fn may(tree: &Operator, query: Option<&TaskQuery>, who: TaskId, permit: Permit) -> Code {
    verdict(&Court { query, tree }, who, permit)
}
