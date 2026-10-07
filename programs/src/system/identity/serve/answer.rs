//! Face admission precedes every state read or write.
use crate::system::identity::core::{Anchor, IdentityBook};
use env::TaskId;
use system_api::identity::{Fail, Grant, PageTarget, Reply, Wire};

pub struct Request {
    pub from: TaskId,
    pub grant: Grant,
    pub wire: Option<Wire>,
}

pub fn answer(book: &mut IdentityBook, request: Request) -> Reply {
    let Request { from, grant, wire } = request;
    let Some(wire) = wire else {
        return Reply::Fail(Fail::Bad);
    };
    if Grant::for_wire(&wire) != grant {
        return Reply::Fail(Fail::Denied);
    }
    let result = match wire {
        Wire::Resolve(task) => Ok(Reply::Binding(book.resolve(task))),
        Wire::Matches(task, selector) => book.matches(task, selector).map(Reply::Match),
        Wire::Same(a, b) => Ok(Reply::Match(book.same(a, b))),
        Wire::Sire(p) => book.sire(p).map(Reply::Principal),
        Wire::Heir(a, b) => book.heir(a, b).map(Reply::Bool),
        Wire::Amid(p, c) => book.amid(p, c).map(Reply::Bool),
        Wire::Members(c, cursor) => book
            .page(PageTarget::Members(c), cursor)
            .map(Reply::Members),
        Wire::Memberships(p, cursor) => book
            .page(PageTarget::Memberships(p), cursor)
            .map(Reply::Memberships),
        Wire::Adopt(s) => book
            .narrow_own(
                from,
                crate::system::identity::core::Selection {
                    subject: s,
                    anchor: Anchor::Keep,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Waive => book.waive(from).map(|()| Reply::Unit),
        Wire::Restrict(s) => book
            .narrow_own(
                from,
                crate::system::identity::core::Selection {
                    subject: s,
                    anchor: Anchor::Move,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Derive(p) => book.derive(from, p).map(|p| Reply::Principal(Some(p))),
        Wire::Found => book.found(from).map(Reply::Coalition),
        Wire::Admit(c, p) => book
            .admit(
                from,
                crate::system::identity::core::Membership {
                    coalition: c,
                    principal: p,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Expel(c, p) => book
            .expel(
                from,
                crate::system::identity::core::Membership {
                    coalition: c,
                    principal: p,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Bind(task, install) => book
            .bind(
                from,
                crate::system::identity::core::BindingRequest {
                    task,
                    install: install,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Unbind(task) => book.unbind(from, task).map(|()| Reply::Unit),
    };
    result.unwrap_or_else(Reply::Fail)
}
