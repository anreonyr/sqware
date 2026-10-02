use super::*;
use protocol::service::identity::{
    CoalitionSet, Cursor, Install, Match, PageTarget, Selector, Subject, limits,
};

fn subject(p: PrincipalId, coalitions: &[CoalitionId]) -> Subject {
    Subject { principal: p, coalitions: CoalitionSet::new(coalitions).unwrap() }
}

fn setup() -> (IdentityBook, TaskId, TaskId, PrincipalId) {
    let installer = TaskId::new(1);
    let task = TaskId::new(3);
    let mut book = IdentityBook::new(TaskId::new(2), installer).unwrap();
    let p = book.derive(installer, book.root()).unwrap();
    book.bind(installer, task, Install::Authorized(subject(p, &[]))).unwrap();
    (book, installer, task, p)
}

#[test]
fn inheritance_uses_current_and_restriction_is_permanent() {
    let (mut book, installer, task, p) = setup();
    let q = book.derive(task, p).unwrap();
    book.adopt(task, subject(q, &[])).unwrap();
    let child = TaskId::new(4);
    book.bind(installer, child, Install::Inherit { parent: task }).unwrap();
    assert_eq!(book.resolve(child).unwrap().origin.principal, q);
    book.waive(task).unwrap();
    assert_eq!(book.resolve(task).unwrap().current.principal, p);
    assert_eq!(book.adopt(child, subject(p, &[])), Err(Fail::NotNarrower));
    book.restrict(task, subject(q, &[])).unwrap();
    book.waive(task).unwrap();
    assert_eq!(book.resolve(task).unwrap().current.principal, q);
}

#[test]
fn eligibility_is_not_active_and_revocation_does_not_reactivate() {
    let (mut book, installer, task, p) = setup();
    let c = book.found(task).unwrap();
    book.admit(task, c, p).unwrap();
    assert_eq!(book.matches(task, Selector::MemberOf(c)), Ok(Match::No));
    book.bind(installer, task, Install::Authorized(subject(p, &[c]))).unwrap();
    assert_eq!(book.matches(task, Selector::MemberOf(c)), Ok(Match::Yes));
    book.expel(task, c, p).unwrap();
    book.admit(task, c, p).unwrap();
    book.waive(task).unwrap();
    assert_eq!(book.matches(task, Selector::MemberOf(c)), Ok(Match::No));
    assert!(book.resolve(task).unwrap().origin.coalitions.as_slice().is_empty());
}

#[test]
fn install_is_sender_anchored_and_failures_leave_binding_unchanged() {
    let (mut book, installer, task, p) = setup();
    let original = book.resolve(task);
    assert_eq!(book.bind(task, task, Install::Authorized(subject(book.root(), &[]))),
        Err(Fail::Denied));
    let c = book.found(task).unwrap();
    assert_eq!(book.bind(installer, task, Install::Authorized(subject(p, &[c]))),
        Err(Fail::NotEligible));
    assert_eq!(book.resolve(task), original);
    assert_eq!(book.unbind(task, task), Err(Fail::Denied));
    book.unbind(installer, task).unwrap();
    book.unbind(installer, task).unwrap();
    assert!(book.resolve(task).is_none());
    assert_eq!(book.sire(p), Ok(Some(book.root())));
}

#[test]
fn managers_are_exact_and_can_manage_offline_principals() {
    let (mut book, installer, task, p) = setup();
    let c = book.found(task).unwrap();
    let descendant = book.derive(task, p).unwrap();
    let offline = book.derive(installer, book.root()).unwrap();
    book.admit(task, c, offline).unwrap();
    assert_eq!(book.amid(offline, c), Ok(true));
    book.adopt(task, subject(descendant, &[])).unwrap();
    assert_eq!(book.expel(task, c, offline), Err(Fail::NotManager));
    book.expel(installer, c, offline).unwrap();
    assert_eq!(book.amid(offline, c), Ok(false));
}

#[test]
fn authority_and_cursor_revision_are_checked_before_results() {
    let (mut book, installer, task, p) = setup();
    let c = book.found(task).unwrap();
    let cursor = Cursor { target: PageTarget::Members(c), revision: book.revision, after: 0 };
    book.admit(task, c, p).unwrap();
    assert!(matches!(book.members(c, Some(cursor)), Err(Fail::Changed)));
    let alien = PrincipalId { authority: installer, slot: p.slot };
    assert_eq!(book.matches(TaskId::new(999), Selector::Exact(alien)),
        Err(Fail::WrongAuthority));
    assert_eq!(book.matches(TaskId::new(999), Selector::Exact(p)), Ok(Match::Unbound));
    book.revision = u64::MAX;
    let before = book.resolve(task);
    assert_eq!(book.expel(task, c, p), Err(Fail::Full));
    assert_eq!(book.amid(p, c), Ok(true));
    assert_eq!(book.resolve(task), before);
}

#[test]
fn revocation_of_origin_intersects_descendant_current_but_current_revocation_can_waive() {
    let (mut book, installer, task, p) = setup();
    let c = book.found(task).unwrap();
    let q = book.derive(task, p).unwrap();
    book.admit(task, c, p).unwrap();
    book.admit(task, c, q).unwrap();
    book.bind(installer, task, Install::Authorized(subject(p, &[c]))).unwrap();
    book.adopt(task, subject(q, &[c])).unwrap();
    book.expel(installer, c, q).unwrap();
    assert_eq!(book.matches(task, Selector::MemberOf(c)), Ok(Match::No));
    book.waive(task).unwrap();
    assert_eq!(book.matches(task, Selector::MemberOf(c)), Ok(Match::Yes));
    book.admit(installer, c, q).unwrap();
    book.adopt(task, subject(q, &[c])).unwrap();
    book.expel(installer, c, p).unwrap();
    assert!(book.resolve(task).unwrap().current.coalitions.as_slice().is_empty());
    assert_eq!(book.amid(q, c), Ok(true));
    book.admit(installer, c, p).unwrap();
    book.waive(task).unwrap();
    assert_eq!(book.matches(task, Selector::MemberOf(c)), Ok(Match::No));
}

#[test]
fn pages_include_slot_zero_and_sort_with_explicit_continuations() {
    let (mut book, installer, task, p) = setup();
    let c = book.found(task).unwrap();
    let root = book.root();
    book.admit(installer, c, root).unwrap();
    // Insert in descending order so the membership storage order cannot masquerade as paging order.
    let mut ids = Vec::new();
    let mut parent = p;
    for _ in 0..limits::PAGE_ITEMS + 1 {
        parent = book.derive(installer, parent).unwrap();
        ids.push(parent);
    }
    for &id in ids.iter().rev() { book.admit(installer, c, id).unwrap(); }
    let first = book.members(c, None).unwrap();
    assert_eq!(first.as_slice()[0], root);
    assert!(first.as_slice().windows(2).all(|w| w[0].slot < w[1].slot));
    let second = book.members(c, first.next()).unwrap();
    assert!(second.as_slice()[0].slot > first.as_slice().last().unwrap().slot);
    assert!(second.next().is_none());
}

#[test]
fn creation_quotas_fail_without_partial_nodes() {
    let (mut book, installer, task, p) = setup();
    for _ in 0..limits::MAX_CREATED_PER_PRINCIPAL {
        book.derive(installer, p).unwrap();
    }
    let before = book.principals.len();
    assert_eq!(book.derive(installer, p), Err(Fail::Full));
    assert_eq!(book.principals.len(), before);
    for _ in 0..limits::MAX_CREATED_PER_PRINCIPAL {
        book.found(task).unwrap();
    }
    let before = book.coalitions.len();
    assert_eq!(book.found(task), Err(Fail::Full));
    assert_eq!(book.coalitions.len(), before);
}
