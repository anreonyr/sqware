//! Face admission precedes every state read or write.
use env::TaskId;
use protocol::service::identity::{Fail, Grant, Reply, Wire};
use crate::service::identity::core::IdentityBook;

pub fn answer(book: &mut IdentityBook, from: TaskId, grant: Grant, wire: Option<Wire>) -> Reply {
    let Some(wire) = wire else { return Reply::Fail(Fail::Bad); };
    if Grant::for_wire(&wire) != grant { return Reply::Fail(Fail::Denied); }
    let result = match wire {
        Wire::Resolve(task) => Ok(Reply::Binding(book.resolve(task))),
        Wire::Matches(task, selector) => book.matches(task, selector).map(Reply::Match),
        Wire::Same(a, b) => Ok(Reply::Match(book.same(a, b))),
        Wire::Sire(p) => book.sire(p).map(Reply::Principal),
        Wire::Heir(a, b) => book.heir(a, b).map(Reply::Bool),
        Wire::Amid(p, c) => book.amid(p, c).map(Reply::Bool),
        Wire::Members(c, cursor) => book.members(c, cursor).map(Reply::Members),
        Wire::Memberships(p, cursor) => book.memberships(p, cursor).map(Reply::Memberships),
        Wire::Adopt(s) => book.adopt(from, s).map(|()| Reply::Unit),
        Wire::Waive => book.waive(from).map(|()| Reply::Unit),
        Wire::Restrict(s) => book.restrict(from, s).map(|()| Reply::Unit),
        Wire::Derive(p) => book.derive(from, p).map(|p| Reply::Principal(Some(p))),
        Wire::Found => book.found(from).map(Reply::Coalition),
        Wire::Admit(c, p) => book.admit(from, c, p).map(|()| Reply::Unit),
        Wire::Expel(c, p) => book.expel(from, c, p).map(|()| Reply::Unit),
        Wire::Bind(task, install) => book.bind(from, task, install).map(|()| Reply::Unit),
        Wire::Unbind(task) => book.unbind(from, task).map(|()| Reply::Unit),
    };
    result.unwrap_or_else(Reply::Fail)
}
