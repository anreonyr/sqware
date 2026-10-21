//! Face admission precedes every state read or write.
use crate::system::identity::book::{
    Anchor, Association, IdentityBook, InstallAction, InstallationRequest, ReferenceQuery,
};
use env::PieToken;
use env::TaskId;
use system_api::identity::Fail;
use system_api::identity::Grant;
use system_api::identity::PageTarget;
use system_api::identity::Reply;
use system_api::identity::Wire;

pub struct Request {
    pub context: Context,
    pub wire: Option<Wire>,
}
pub struct Context {
    pub from: TaskId,
    pub endpoint: PieToken,
    pub grant: Grant,
}

pub fn answer(book: &mut IdentityBook, request: Request) -> Reply {
    let Request {
        context: Context {
            from,
            endpoint,
            grant,
        },
        wire,
    } = request;
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
                crate::system::identity::book::Selection {
                    subject: s,
                    anchor: Anchor::Keep,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Waive => book.waive(from).map(|()| Reply::Unit),
        Wire::Restrict(s) => book
            .narrow_own(
                from,
                crate::system::identity::book::Selection {
                    subject: s,
                    anchor: Anchor::Move,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Derive(p) => book.derive(from, p).map(|p| Reply::Principal(Some(p))),
        Wire::Found(id) => book.found(from, id).map(Reply::Coalition),
        Wire::Admit(c, p) => book
            .admit(
                from,
                crate::system::identity::book::Membership {
                    coalition: c,
                    principal: p,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Expel(c, p) => book
            .expel(
                from,
                crate::system::identity::book::Membership {
                    coalition: c,
                    principal: p,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Bind(task, install) => book
            .bind(
                from,
                crate::system::identity::book::BindingRequest {
                    task,
                    install: install,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Unbind(task) => book.unbind(from, task).map(|()| Reply::Unit),
        Wire::Activate(task, coalitions) => book
            .activate(from, (task, coalitions))
            .map(|()| Reply::Unit),
        Wire::Install(id, spec) => book
            .installation(InstallationRequest {
                endpoint,
                caller: from,
                id,
                action: InstallAction::Apply(spec),
            })
            .map(Reply::Installation),
        Wire::Inspect(id) => book
            .installation(InstallationRequest {
                endpoint,
                caller: from,
                id,
                action: InstallAction::Inspect,
            })
            .map(Reply::Installation),
        Wire::Cancel(id) => book
            .installation(InstallationRequest {
                endpoint,
                caller: from,
                id,
                action: InstallAction::Cancel,
            })
            .map(Reply::Installation),
        Wire::Retire(id) => book
            .installation(InstallationRequest {
                endpoint,
                caller: from,
                id,
                action: InstallAction::Retire,
            })
            .map(Reply::Installation),
        Wire::Associate(object, name, entry) => book
            .associate(
                from,
                Association {
                    object,
                    name,
                    entry,
                },
            )
            .map(|()| Reply::Unit),
        Wire::Reference(object, name) => book
            .reference(ReferenceQuery { object, name })
            .map(Reply::Reference),
    };
    result.unwrap_or_else(Reply::Fail)
}
