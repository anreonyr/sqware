#![no_std]
#![no_main]

//! Organization eligibility is not active identity. Only the exact manager may mutate it.

extern crate programs;

use env::Wait;
use programs::Report;
use protocol::communication::session::Session;
use protocol::service::identity::client::{CallError, Organization, Query, SelfOps};
use protocol::service::identity::{CoalitionId, Fail, Match, Selector, Subject};
use protocol::service::operator::client as operator;
use runtime::env::unit as utask;

const MS: usize = 1000;

#[programs::entry]
fn main() -> Report<'static> {
    let authority = programs::service::identity::bridge::authority()
        .expect("member: no Control-issued identity authority");
    let session = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS))
        .ok()
        .expect("member: no tree link");
    let tree = operator::Face::of(session);
    let query = Query::discover(&tree, authority, Wait::AtMost(MS)).expect("member: no query");
    let own =
        SelfOps::discover(&tree, authority, Wait::AtMost(MS)).expect("member: no self actions");
    let org = Organization::discover(&tree, authority, Wait::AtMost(MS))
        .expect("member: no organization actions");
    let me = utask::self_id();
    let initial = query
        .resolve(me, Wait::AtMost(MS))
        .unwrap()
        .expect("member: unbound");
    let p = initial.current.principal;
    let c0 = org
        .found(Wait::AtMost(MS))
        .expect("member: found c0 failed");
    let c1 = org
        .found(Wait::AtMost(MS))
        .expect("member: found c1 failed");
    assert_eq!(c0.authority, authority);
    assert_eq!(c1.authority, authority);
    assert!(c1.slot > c0.slot);
    assert_eq!(query.amid(p, c0, Wait::AtMost(MS)), Ok(false));

    org.admit(c0, p, Wait::AtMost(MS))
        .expect("member: manager admit failed");
    org.admit(c0, p, Wait::AtMost(MS))
        .expect("member: repeated admit failed");
    org.admit(c1, p, Wait::AtMost(MS))
        .expect("member: second admit failed");
    assert_eq!(query.amid(p, c0, Wait::AtMost(MS)), Ok(true));
    assert_eq!(query.amid(p, c1, Wait::AtMost(MS)), Ok(true));
    assert_eq!(
        query.matches(me, Selector::MemberOf(c0), Wait::AtMost(MS)),
        Ok(Match::No),
        "admit must not activate membership"
    );
    assert!(
        matches!(
            own.adopt(Subject::new(p, &[c0]).unwrap(), Wait::AtMost(MS)),
            Err(CallError::Service(Fail::NotNarrower))
        ),
        "adopt must not add a coalition outside current/origin"
    );
    assert_eq!(query.resolve(me, Wait::AtMost(MS)), Ok(Some(initial)));

    let q = own
        .derive(p, Wait::AtMost(MS))
        .expect("member: derive failed");
    org.admit(c0, q, Wait::AtMost(MS))
        .expect("member: offline subject admit failed");
    assert_eq!(query.amid(q, c0, Wait::AtMost(MS)), Ok(true));
    let page = query
        .members(c0, None, Wait::AtMost(MS))
        .expect("member: members failed");
    assert!(page.as_slice().contains(&p));
    assert!(page.as_slice().contains(&q));
    let memberships = query
        .memberships(p, None, Wait::AtMost(MS))
        .expect("member: memberships failed");
    assert!(memberships.as_slice().contains(&c0));
    assert!(memberships.as_slice().contains(&c1));

    own.adopt(Subject::new(q, &[]).unwrap(), Wait::AtMost(MS))
        .expect("member: adopt failed");
    assert!(
        matches!(
            org.admit(c0, q, Wait::AtMost(MS)),
            Err(CallError::Service(Fail::NotManager))
        ),
        "a manager descendant must not inherit management"
    );
    assert!(matches!(
        org.expel(c0, p, Wait::AtMost(MS)),
        Err(CallError::Service(Fail::NotManager))
    ));
    assert_eq!(
        query.amid(p, c0, Wait::AtMost(MS)),
        Ok(true),
        "denied expel must not change eligibility"
    );
    assert_eq!(
        query.matches(me, Selector::MemberOf(c0), Wait::AtMost(MS)),
        Ok(Match::No)
    );
    own.waive(Wait::AtMost(MS)).expect("member: waive failed");
    org.expel(c0, q, Wait::AtMost(MS))
        .expect("member: expel failed");
    org.expel(c0, q, Wait::AtMost(MS))
        .expect("member: repeated expel failed");
    assert_eq!(query.amid(q, c0, Wait::AtMost(MS)), Ok(false));
    assert_eq!(query.amid(p, c0, Wait::AtMost(MS)), Ok(true));
    org.admit(c0, q, Wait::AtMost(MS))
        .expect("member: readmit failed");
    assert_eq!(
        query.matches(me, Selector::MemberOf(c0), Wait::AtMost(MS)),
        Ok(Match::No)
    );

    // Fill only until the service supplies a continuation. Mutating the relation must
    // invalidate that revision-bound cursor rather than silently continuing a new list.
    let mut cursor = None;
    let mut last = q;
    for _ in 0..=protocol::service::identity::limits::PAGE_ITEMS {
        let page = query
            .members(c0, None, Wait::AtMost(MS))
            .expect("member: page failed");
        if page.next().is_some() {
            cursor = page.next();
            break;
        }
        last = own
            .derive(p, Wait::AtMost(MS))
            .expect("member: page subject derive failed");
        org.admit(c0, last, Wait::AtMost(MS))
            .expect("member: page subject admit failed");
    }
    let cursor = cursor.expect("member: no continuation within acceptance bound");
    org.expel(c0, last, Wait::AtMost(MS))
        .expect("member: cursor mutation failed");
    assert!(matches!(
        query.members(c0, Some(cursor), Wait::AtMost(MS)),
        Err(CallError::Service(Fail::Changed))
    ));
    assert!(query.members(c0, None, Wait::AtMost(MS)).is_ok());

    let outside = CoalitionId::new(authority, u64::MAX);
    assert!(matches!(
        query.amid(p, outside, Wait::AtMost(MS)),
        Err(CallError::Service(Fail::UnknownCoalition))
    ));
    assert!(matches!(
        org.admit(outside, p, Wait::AtMost(MS)),
        Err(CallError::Service(Fail::UnknownCoalition))
    ));
    Report::note(
        env::EXIT_OK,
        "member: manager, inactive selection and pages held",
    )
}
